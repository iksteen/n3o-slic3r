//! Slice job model + registry.
//!
//! The orchestrator's input shape, opaque handle, status snapshot,
//! and the registry that owns in-flight jobs. The `slice_active_plate`
//! command builds a [`SliceJobInput`] from the active plate's scene +
//! cascade selection, runs it through the orchestrator, returns a
//! [`JobId`], and drives the lifecycle via Tauri events plus
//! [`slice_status`] for reconnect.
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

use crate::core::cascade::commands::ContextJson;

/// Opaque monotonic job id. 1-based; 0 is reserved as "no job".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub u64);

/// The orchestrator's resolved input for one slice job, built by
/// [`super::input::build_slice_input`] from the active plate.
/// `context` is the serialized cascade context ([`ContextJson`]).
#[derive(Debug, Clone, Deserialize)]
pub struct SliceJobInput {
    /// The plate's geometry, one entry per scene object, fed to
    /// libslic3r's `Model` straight from the in-memory mesh buffers
    /// (no temp `.3mf` round-trip). Built by
    /// [`super::input::build_plate_objects`]. `#[serde(skip)]` because
    /// the buffers are `Arc`-shared, backend-only data that never crosses
    /// the IPC bridge — a frontend-posted `SliceJobInput` deserializes to
    /// an empty object list (the backend always rebuilds it from project
    /// state in `build_slice_input`).
    #[serde(skip)]
    pub objects: Vec<super::input::SliceObject>,
    /// Test/parity only: round-trip the geometry through a temp `.3mf`
    /// (`write_3mf` → `Model::load`) instead of building the `Model`
    /// in-memory via `Model::add_object`. Default `false` = buffer-load.
    /// `build_slice_input` also sets this `true` for plates carrying
    /// multi-volume groups, which the single-mesh `add_object` FFI can't
    /// represent — the temp-`.3mf` writer collapses each group into one
    /// `ModelObject` with N volumes (the floating-regions fix).
    #[serde(default)]
    pub force_temp_3mf: bool,
    /// Directory to write G-code into. Output files land at
    /// `<output_dir>/plate_<N>.gcode`.
    pub output_dir: String,
    pub context: ContextJson,
    /// Plates to slice. MVP iterates this sequentially; multi-plate
    /// projects are Phase 5.
    pub plate_ids: Vec<u32>,
    /// Names the PrinterInstance whose per-bucket fragments compose
    /// this job's cascade. The orchestrator looks the instance up
    /// in the bundled printer library and composes a fresh cascade
    /// per job — no shared registry, no preloaded monolithic
    /// cascade.
    pub printer_instance_id: String,
    /// Per-material slot bindings: one entry per model material on
    /// the plate, in material-index order (material `i + 1` at
    /// position `i`). `None` for materials with no slot binding
    /// yet. The composer fans the libslic3r filament dimension
    /// (filament_diameter, filament_colour, filament_map, …) out
    /// to this length, and each [`SliceObject::extruder`] carries the
    /// 1-based material number (post material→filament remap) so the
    /// gcode emits `T<material - 1>` for each cube. The driver's
    /// `ams_mapping` array uses the same indexing.
    ///
    /// Empty when the plate has no materials (an empty plate's
    /// slice job fails earlier, but the field is included for
    /// the legacy Deserialize path used by frontend-posted
    /// SliceJobInput shapes).
    #[serde(default)]
    pub material_layout: Vec<Option<crate::core::printer::SlotRef>>,
    /// The plate's process/quality profile override (a bundled
    /// process-fragment slug), or `None` to inherit the bound
    /// instance's `quality_profile`. The orchestrator composes the
    /// cascade against this effective process.
    #[serde(default)]
    pub quality_profile: Option<String>,
    /// MMU color-paint filament remap for toolchanger printers, or `None`
    /// when no remap is needed (AMS printers, or a plate with no painting).
    /// `perm[state]` is the libslic3r filament index the painted state should
    /// route to — the same `material → flat-slot` remap the per-object
    /// `extruder_id` gets, so painted faces follow the base material onto the
    /// right toolhead. The orchestrator applies it via
    /// `Model::remap_paint_filaments` after building the model.
    #[serde(default)]
    pub paint_filament_remap: Option<Vec<i32>>,
}

/// Snapshot of a job's lifecycle. Returned by `slice_status` so the
/// renderer can rebuild progress UI after a reconnect — the events
/// were already emitted but a fresh renderer wasn't listening.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "data")]
pub enum JobStatus {
    /// Queued but worker hasn't picked it up yet. Brief window —
    /// today the worker thread spawns synchronously inside the
    /// orchestrator's start path.
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
    Failed { plate_id: u32, error: String },
    /// User cancelled.
    Cancelled { plate_id_in_progress: Option<u32> },
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

    /// Drop the handle from the registry. Not currently wired into the
    /// worker lifecycle: the registry is never pruned, so completed
    /// handles accumulate for the lifetime of the process (pruning
    /// after terminal events is a separate code task). The UI reads
    /// terminal events from the Tauri event channel rather than
    /// polling; once a handle is removed, the snapshot read via
    /// `slice_status` errors with "no such job".
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
    /// Plate geometry fed to libslic3r — see [`SliceJobInput::objects`].
    pub objects: Vec<super::input::SliceObject>,
    /// See [`SliceJobInput::force_temp_3mf`].
    pub force_temp_3mf: bool,
    pub output_dir: PathBuf,
    pub plate_ids: Vec<u32>,
    pub cascade: crate::core::cascade::Cascade,
    /// The user/project/object override tiers, parsed from the slice
    /// context's override specs at job prep. Applied on top of `cascade`
    /// per plate by `cascade::resolve_with_overrides` (the second phase).
    pub override_tiers: crate::core::cascade::OverrideTiers,
    pub context: crate::core::project::SlicingContext,
    /// Bound filament loadout (slice-time material→slot mapping),
    /// snapshotted from the PrinterInstance at job prep and handed to
    /// the pre/post-slice plugin hooks. Empty when the instance can't
    /// be resolved (plugins then see no slots — offline-safe).
    pub filament: crate::core::plugin::FilamentLoadout,
    /// Flat `plugin.*` override entries for the **project** level
    /// (cascade *user* tier, `Project.user_overrides`) and the **plate**
    /// level (cascade *project* tier, `Plate.project_overrides`),
    /// extracted at prep. Fed to the per-plate `DispatchGate`; the host
    /// resolves each plugin's activation + settings from them plus the
    /// global tier.
    /// Flat `plugin.*` entries for the **printer-instance** tier — the
    /// bound instance's `config_overrides` (a per-printer default the
    /// project/plate tiers override). Snapshotted at job prep.
    pub plugin_instance: std::collections::BTreeMap<String, String>,
    pub plugin_project: std::collections::BTreeMap<String, String>,
    pub plugin_plate: std::collections::BTreeMap<String, String>,
    /// MMU paint filament remap (toolchanger only); see
    /// [`SliceJobInput::paint_filament_remap`]. Applied to the loaded model
    /// before slicing.
    pub paint_filament_remap: Option<Vec<i32>>,
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
