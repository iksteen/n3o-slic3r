//! Slice event payloads.
//!
//! Mirrors the `scene:*` event-stream pattern: the orchestrator's
//! worker thread emits a typed event per lifecycle transition + per
//! progress tick. The frontend subscribes via Tauri's `listen` and
//! threads the events into its slice-panel state.
//!
//! Names follow `slice:<noun>_<verb>` (snake_case). See [`name`] for
//! the canonical name string per variant.

use serde::Serialize;

use super::errors::SliceError;
use super::job::JobId;
use super::summary::PlateSummary;

/// The prime/wipe tower's exact mesh from the slice (the rib/cone solid
/// for toolchangers, a box for AMS purge towers), in tower-local
/// millimetres: `vertices` is 3 floats per vertex, `indices` 3 vertex
/// indices per triangle. The viewport renders it at the plate's
/// `wipe_tower_x/y`. `None` for a single-material plate (no tower).
#[derive(Debug, Clone, Serialize)]
pub struct TowerMesh {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "data")]
pub enum SliceEvent {
    PlateStarted {
        job_id: JobId,
        plate_id: u32,
    },
    /// Throttled — emitted at most once per 50 ms per plate unless
    /// the stage label changed (in which case it fires immediately
    /// so the user feels phase transitions). The orchestrator does
    /// the throttling; libslic3r can emit hundreds of ticks per
    /// second.
    PlateProgress {
        job_id: JobId,
        plate_id: u32,
        percent: i32,
        stage: String,
    },
    PlateFinished {
        job_id: JobId,
        plate_id: u32,
        output_path: String,
        summary: PlateSummary,
        /// The prime/wipe tower mesh libslic3r built for this plate, or
        /// `None` when single-material (no tower). The viewport shows the
        /// exact shape; it survives drags (placed at the live
        /// `wipe_tower_x/y`) and goes stale only on a material-count change.
        tower_mesh: Option<TowerMesh>,
    },
    JobFinished {
        job_id: JobId,
    },
    JobFailed {
        job_id: JobId,
        plate_id: u32,
        error: SliceError,
    },
    /// User cancelled. `plate_id_in_progress` names whichever plate
    /// the worker was on when the cancel fired (None if no plate
    /// had started yet — corner case).
    Cancelled {
        job_id: JobId,
        plate_id_in_progress: Option<u32>,
    },
}

impl SliceEvent {
    /// Tauri event channel name. Frontend subscribes per name; the
    /// payload is the SceneEvent JSON the `Serialize` derive emits.
    pub fn name(&self) -> &'static str {
        match self {
            Self::PlateStarted { .. } => "slice:plate_started",
            Self::PlateProgress { .. } => "slice:plate_progress",
            Self::PlateFinished { .. } => "slice:plate_finished",
            Self::JobFinished { .. } => "slice:job_finished",
            Self::JobFailed { .. } => "slice:job_failed",
            Self::Cancelled { .. } => "slice:cancelled",
        }
    }
}
