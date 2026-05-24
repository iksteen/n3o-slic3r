//! Slice job model + registry (PR-3-2).
//!
//! The orchestrator's input shape, opaque handle, status snapshot,
//! and the registry that owns in-flight jobs. The frontend builds a
//! [`SliceJobInput`] from its current scene + cascade selection,
//! sends it through `slice_start_job`, gets back a [`JobId`], and
//! drives the lifecycle via Tauri events plus [`slice_status`] for
//! reconnect.
//!
//! Cancellation is cooperative: each job carries an `Arc<AtomicBool>`
//! the worker thread polls between plates (and in the future, between
//! libslic3r progress ticks once that's wired through the FFI's
//! cancel hook). `slice_cancel` flips the flag; the worker emits a
//! `slice:cancelled` event on next check.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::core::cascade::commands::{CascadeHandle, ContextJson};

/// Opaque monotonic job id. 1-based; 0 is reserved as "no job".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub u64);

/// What the frontend hands to `slice_start_job`.
///
/// `context` is the same shape the cascade-command surface accepts —
/// the frontend already builds these for `cascade_resolve` /
/// `cascade_trace`, so wire format stays consistent.
#[derive(Debug, Clone, Deserialize)]
pub struct SliceJobInput {
    /// Filesystem path of the model to slice. For MVP this is a
    /// pre-existing file (STL / OBJ / 3MF) — the scene→file bridge
    /// the UI uses is `scene_load_*` on the way in, and a future
    /// PR-3-2 refinement will dump the live scene to a temp 3MF
    /// via PR-3-9's writer so the user's plate-arranged scene
    /// reaches libslic3r without round-tripping through disk
    /// manually.
    pub model_path: String,
    /// Directory to write G-code into. Output files land at
    /// `<output_dir>/plate_<N>.gcode`.
    pub output_dir: String,
    pub cascade_handle: CascadeHandle,
    pub context: ContextJson,
    /// Plates to slice. MVP iterates this sequentially; multi-plate
    /// projects are Phase 5.
    pub plate_ids: Vec<u32>,
    /// PR-S-5b: if set, the orchestrator composes a fresh cascade from
    /// this PrinterInstance's per-bucket fragments + the plate's
    /// process overrides, then uses *that* cascade instead of looking
    /// up `cascade_handle` in the registry. `None` falls back to the
    /// legacy monolithic-cascade path (PR-S-5c will rip this fallback
    /// out + make composition the only path).
    #[serde(default)]
    pub printer_instance_id: Option<String>,
}

/// Snapshot of a job's lifecycle. Returned by `slice_status` so the
/// renderer can rebuild progress UI after a reconnect — the events
/// were already emitted but a fresh renderer wasn't listening.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "data")]
pub enum JobStatus {
    /// Queued but worker hasn't picked it up yet. Brief window —
    /// today the worker thread spawns synchronously inside
    /// `slice_start_job`.
    Queued,
    /// Worker is processing a plate. `percent` / `stage` reflect the
    /// most recent libslic3r progress tick.
    Running {
        plate_id: u32,
        percent: i32,
        stage: String,
    },
    /// Cancel flag is set; worker hasn't yet emitted the
    /// `slice:cancelled` event. Transient.
    Cancelling,
    /// All plates finished cleanly.
    Finished,
    /// A plate failed. `plate_id` names the offending plate;
    /// `error` is a string representation of the typed
    /// `SliceError` (the typed value already rode out via the
    /// `slice:job_failed` event).
    Failed {
        plate_id: u32,
        error: String,
    },
    /// User cancelled.
    Cancelled {
        plate_id_in_progress: Option<u32>,
    },
}

/// Shared handle between the orchestrator command thread and the
/// worker thread. The Mutex is short-held — worker takes it only to
/// update `status` (a tiny enum); commands take it to read.
pub struct JobHandle {
    pub cancel: Arc<AtomicBool>,
    pub status: std::sync::Mutex<JobStatus>,
}

impl JobHandle {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            cancel: Arc::new(AtomicBool::new(false)),
            status: std::sync::Mutex::new(JobStatus::Queued),
        })
    }

    pub fn set_status(&self, status: JobStatus) {
        if let Ok(mut guard) = self.status.lock() {
            *guard = status;
        }
    }

    pub fn snapshot(&self) -> JobStatus {
        self.status
            .lock()
            .map(|g| g.clone())
            .unwrap_or(JobStatus::Queued)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.set_status(JobStatus::Cancelling);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
}

/// Tauri-managed state. Wraps the in-flight job table behind a
/// short-held Mutex. Job lookup is O(1) on a `HashMap<JobId,
/// Arc<JobHandle>>`; the registry never holds the lock across the
/// worker thread's runtime.
pub struct JobRegistry {
    next_id: AtomicU64,
    jobs: std::sync::Mutex<HashMap<JobId, Arc<JobHandle>>>,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self {
            // Start at 1 so a default-constructed JobId(0) is
            // distinguishable from a real one.
            next_id: AtomicU64::new(1),
            jobs: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn alloc_id(&self) -> JobId {
        JobId(self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    pub fn insert(&self, id: JobId, handle: Arc<JobHandle>) {
        if let Ok(mut guard) = self.jobs.lock() {
            guard.insert(id, handle);
        }
    }

    pub fn get(&self, id: JobId) -> Option<Arc<JobHandle>> {
        self.jobs.lock().ok()?.get(&id).cloned()
    }

    /// Drop the handle. Called by the worker thread after it emits a
    /// terminal event (finished / failed / cancelled) so the
    /// registry doesn't accumulate completed jobs across a long
    /// session. Tests + the UI can still read the terminal event
    /// from the Tauri event channel; the snapshot read via
    /// `slice_status` simply errors with "no such job" after
    /// removal.
    pub fn remove(&self, id: JobId) {
        if let Ok(mut guard) = self.jobs.lock() {
            guard.remove(&id);
        }
    }
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolved input the worker thread holds. Built once from
/// [`SliceJobInput`] by the orchestrator entry so the worker
/// doesn't share lock-held cascade refs with the command thread.
pub struct ResolvedJob {
    pub model_path: PathBuf,
    pub output_dir: PathBuf,
    pub plate_ids: Vec<u32>,
    pub cascade: crate::core::cascade::Cascade,
    pub context: crate::core::project::SlicingContext,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_allocates_monotonic_ids() {
        let r = JobRegistry::new();
        let a = r.alloc_id();
        let b = r.alloc_id();
        let c = r.alloc_id();
        assert_eq!(a.0, 1);
        assert_eq!(b.0, 2);
        assert_eq!(c.0, 3);
    }

    #[test]
    fn registry_insert_get_remove_round_trip() {
        let r = JobRegistry::new();
        let id = r.alloc_id();
        let handle = JobHandle::new();
        r.insert(id, handle.clone());
        let fetched = r.get(id).expect("registered");
        assert!(matches!(fetched.snapshot(), JobStatus::Queued));
        r.remove(id);
        assert!(r.get(id).is_none());
    }

    #[test]
    fn job_handle_cancel_flips_flag_and_status() {
        let h = JobHandle::new();
        assert!(!h.is_cancelled());
        h.cancel();
        assert!(h.is_cancelled());
        assert!(matches!(h.snapshot(), JobStatus::Cancelling));
    }

    #[test]
    fn job_id_serde_round_trip_as_bare_integer() {
        let id = JobId(42);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "42");
        let parsed: JobId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }
}
