//! Scene events — the diff payloads the renderer subscribes to.
//!
//! `SceneState`'s mutation methods (`select`, `translate`, etc.) are
//! *pure*: they take `&mut self` and return a `Vec<SceneEvent>`. The
//! Tauri wrapper in `commands.rs` takes that list and emits each
//! event via `Window::emit`. Tests bypass the Tauri layer and
//! inspect the returned events directly — no mock framework needed.
//!
//! Event names follow `scene:<noun>_<verb>` (e.g.
//! `scene:object_updated`). The frontend's `eventBridge.ts`
//! (PR-2-9) matches on these to update the local mirror.

use super::bed::{BedMesh, OutOfBoundsReason};
use super::state::{
    CameraState, GizmoState, MeshHeader, MeshId, ObjectId, SceneObject,
};
use serde::Serialize;

/// One diff payload the renderer applies to its local mirror.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "data")]
pub enum SceneEvent {
    MeshLoaded(MeshHeader),
    ObjectAdded(SceneObject),
    /// Full updated object — simpler than diff compression for MVP.
    /// PR-2-9 / PR-2-11 can introduce per-field diffs later if the
    /// 5 ms p99 budget is tight.
    ObjectUpdated(SceneObject),
    ObjectRemoved {
        id: ObjectId,
    },
    SelectionChanged {
        selected: Vec<ObjectId>,
    },
    GizmoChanged(GizmoState),
    CameraChanged(CameraState),
    /// Active bed payload changed (printer switch). Renderer
    /// redraws the grid + origin marker + exclusion-zone overlays
    /// from this. None means "no active printer / clear the bed".
    BedChanged(Option<BedMesh>),
    /// Object is currently out of bounds. Non-blocking; the user
    /// fixes it or accepts. Empty `reasons` is impossible (the
    /// scene only emits this on actual violations) but the field
    /// is plural since multiple reasons can apply (off-bed *and*
    /// below z=0).
    ObjectOutOfBounds {
        id: ObjectId,
        reasons: Vec<OutOfBoundsReason>,
    },
    /// Non-uniform scale was just applied (factor components differ).
    /// Non-blocking — the renderer pairs this with the ObjectUpdated
    /// to flag the affected object in the UI, since dimensional
    /// cascade settings (line widths, top-surface thresholds) assume
    /// physical extents and a stretched object skews those.
    NonUniformScale {
        id: ObjectId,
    },
    /// Auto-arrange could not fit every visible object. Non-blocking;
    /// the placed objects still moved. UI flags the listed ids in
    /// the outliner so the user can resize / remove / split to a
    /// new plate (Phase 5).
    AutoArrangeOverflow {
        un_placed: Vec<ObjectId>,
    },
    /// A new plate was added (PR-5-2). `plate_index` is the position
    /// in `SceneState::plates` of the newly-appended entry —
    /// inserts always happen at the end.
    PlateAdded {
        plate_index: usize,
    },
    /// A plate was removed (PR-5-2). `plate_index` is the position
    /// the plate occupied before removal; subsequent plates have
    /// shifted down by one. Pairs with `ActivePlateChanged` when
    /// the removed plate was the active one or sat before it.
    PlateRemoved {
        plate_index: usize,
    },
    /// The active plate changed (PR-5-2). Emitted on explicit
    /// switches and on remove-of-active-or-earlier rebalancing.
    ActivePlateChanged {
        plate_index: usize,
    },
}

impl SceneEvent {
    /// The `scene:*` event name the Tauri layer emits this payload
    /// under. Matches the frontend's `eventBridge.ts` switch
    /// statement in PR-2-9.
    pub fn name(&self) -> &'static str {
        match self {
            Self::MeshLoaded(_) => "scene:mesh_loaded",
            Self::ObjectAdded(_) => "scene:object_added",
            Self::ObjectUpdated(_) => "scene:object_updated",
            Self::ObjectRemoved { .. } => "scene:object_removed",
            Self::SelectionChanged { .. } => "scene:selection_changed",
            Self::GizmoChanged(_) => "scene:gizmo_changed",
            Self::CameraChanged(_) => "scene:camera_changed",
            Self::BedChanged(_) => "scene:bed_changed",
            Self::ObjectOutOfBounds { .. } => "scene:object_out_of_bounds",
            Self::NonUniformScale { .. } => "scene:non_uniform_scale",
            Self::AutoArrangeOverflow { .. } => "scene:auto_arrange_overflow",
            Self::PlateAdded { .. } => "scene:plate_added",
            Self::PlateRemoved { .. } => "scene:plate_removed",
            Self::ActivePlateChanged { .. } => "scene:active_plate_changed",
        }
    }
}

/// Which world-space axis a mirror op reflects across.
#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum MirrorAxis {
    X,
    Y,
    Z,
}

/// How a selection command merges with the existing selection.
#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum SelectMode {
    /// Replace the selection with the given ids.
    Replace,
    /// Add the given ids to the selection.
    Add,
    /// Toggle each id: select if not selected, deselect if selected.
    Toggle,
}

/// Errors mutation methods may return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value")]
pub enum SceneOpError {
    UnknownObject(ObjectId),
    UnknownMesh(MeshId),
    /// Plate index is out of range (PR-5-2).
    UnknownPlate(usize),
    /// Tried to remove the only remaining plate (PR-5-2).
    LastPlate,
}

impl std::fmt::Display for SceneOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownObject(id) => write!(f, "no scene object with id {}", id.0),
            Self::UnknownMesh(id) => write!(f, "no mesh with id {}", id.0),
            Self::UnknownPlate(idx) => write!(f, "no plate at index {idx}"),
            Self::LastPlate => write!(f, "cannot remove the last plate"),
        }
    }
}

impl std::error::Error for SceneOpError {}
