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
use crate::core::project::model::PlateId;
use serde::Serialize;

/// One diff payload the renderer applies to its local mirror.
///
/// **Variant convention (PR-5-2 phase C):** Every variant uses
/// struct-shape fields (not tuple shape) so consumers can pattern
/// match by name and so new fields can land without re-rolling
/// the wire shape. Every plate-scoped variant carries
/// `plate_id: PlateId` as the first field so the frontend mirror
/// can route the event to the right per-plate cache.
///
/// Scene-wide variants (mesh registry, project save/load) don't
/// have `plate_id`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "data")]
pub enum SceneEvent {
    // ---- Scene-wide -------------------------------------------------
    /// A mesh was added to the scene-wide mesh registry. Meshes
    /// live on `Project.meshes` (not per-plate) so this event
    /// carries no `plate_id` — the same mesh can be referenced by
    /// objects on multiple plates.
    MeshLoaded {
        mesh: MeshHeader,
    },

    // ---- Per-plate scene-graph deltas -------------------------------
    ObjectAdded {
        plate_id: PlateId,
        object: SceneObject,
    },
    /// Full updated object — simpler than diff compression for MVP.
    /// PR-2-9 / PR-2-11 can introduce per-field diffs later if the
    /// 5 ms p99 budget is tight.
    ObjectUpdated {
        plate_id: PlateId,
        object: SceneObject,
    },
    ObjectRemoved {
        plate_id: PlateId,
        object_id: ObjectId,
    },
    SelectionChanged {
        plate_id: PlateId,
        selected: Vec<ObjectId>,
    },
    GizmoChanged {
        plate_id: PlateId,
        gizmo: GizmoState,
    },
    CameraChanged {
        plate_id: PlateId,
        camera: CameraState,
    },
    /// A plate's bed payload changed (printer switch on this
    /// plate). Renderer redraws the grid + origin marker +
    /// exclusion-zone overlays from this. `bed: None` means "no
    /// active printer on this plate / clear the bed."
    BedChanged {
        plate_id: PlateId,
        bed: Option<BedMesh>,
    },
    /// Object is currently out of bounds on its plate.
    /// Non-blocking; the user fixes it or accepts. Empty
    /// `reasons` is impossible (the scene only emits this on
    /// actual violations) but the field is plural since multiple
    /// reasons can apply (off-bed *and* below z=0).
    ObjectOutOfBounds {
        plate_id: PlateId,
        object_id: ObjectId,
        reasons: Vec<OutOfBoundsReason>,
    },
    /// Non-uniform scale was just applied to an object (factor
    /// components differ). Non-blocking — the renderer pairs
    /// this with the ObjectUpdated to flag the affected object
    /// in the UI, since dimensional cascade settings (line
    /// widths, top-surface thresholds) assume physical extents
    /// and a stretched object skews those.
    NonUniformScale {
        plate_id: PlateId,
        object_id: ObjectId,
    },
    /// Auto-arrange on a plate could not fit every visible object.
    /// Non-blocking; the placed objects still moved. UI flags the
    /// listed ids in the outliner so the user can resize / remove /
    /// move to a different plate (PR-5-11).
    AutoArrangeOverflow {
        plate_id: PlateId,
        un_placed: Vec<ObjectId>,
    },
    /// A new plate was added (PR-5-2). The frontend mirror
    /// reads `plate_id` to track it; subsequent events on this
    /// plate carry the same id.
    PlateAdded {
        plate_id: PlateId,
    },
    /// A plate was removed (PR-5-2). Pairs with
    /// `ActivePlateChanged` when the removed plate was the active
    /// one (the rebalanced active plate's id ships separately).
    PlateRemoved {
        plate_id: PlateId,
    },
    /// The active plate changed (PR-5-2). Emitted on explicit
    /// switches and on remove-of-active rebalancing.
    ActivePlateChanged {
        plate_id: PlateId,
    },
    /// One or more cascade overrides on a specific object changed
    /// (PR-5-7). The frontend re-runs cascade resolution to refresh
    /// the panel — the event carries no value payload because the
    /// resolver re-reads the override map directly.
    ObjectOverridesChanged {
        plate_id: PlateId,
        object_id: ObjectId,
    },
    /// One or more project-tier overrides on a plate changed
    /// (PR-5-9). Same shape as `ObjectOverridesChanged` minus the
    /// object id — the cascade re-resolves with the updated plate
    /// override map.
    ProjectOverridesChanged {
        plate_id: PlateId,
    },
    /// A plate's metadata changed — cycle count, composition order,
    /// or (post-PR-5-3) name (PR-5-5). The frontend re-reads the
    /// plate's metadata via the project snapshot to refresh the tab
    /// badge / inputs.
    PlateMetadataChanged {
        plate_id: PlateId,
    },
    /// A plate's material bindings changed (PR-5-6). The frontend
    /// re-reads the bindings via the project snapshot to refresh
    /// the binding panel.
    MaterialBindingChanged {
        plate_id: PlateId,
    },
    /// A project was written to disk (PR-5-8). `path` is the
    /// container the writer just produced. UI updates the window
    /// title + recent-files list.
    ProjectSaved {
        path: String,
    },
    /// A project was loaded from disk (PR-5-8) — the in-memory
    /// `Project` state has been replaced wholesale. The frontend
    /// drops every cached plate / mesh / object and re-fetches via
    /// `scene_snapshot`.
    ProjectLoaded {
        path: String,
    },
}

impl SceneEvent {
    /// The `scene:*` event name the Tauri layer emits this payload
    /// under. Matches the frontend's `eventBridge.ts` switch
    /// statement in PR-2-9.
    pub fn name(&self) -> &'static str {
        match self {
            Self::MeshLoaded { .. } => "scene:mesh_loaded",
            Self::ObjectAdded { .. } => "scene:object_added",
            Self::ObjectUpdated { .. } => "scene:object_updated",
            Self::ObjectRemoved { .. } => "scene:object_removed",
            Self::SelectionChanged { .. } => "scene:selection_changed",
            Self::GizmoChanged { .. } => "scene:gizmo_changed",
            Self::CameraChanged { .. } => "scene:camera_changed",
            Self::BedChanged { .. } => "scene:bed_changed",
            Self::ObjectOutOfBounds { .. } => "scene:object_out_of_bounds",
            Self::NonUniformScale { .. } => "scene:non_uniform_scale",
            Self::AutoArrangeOverflow { .. } => "scene:auto_arrange_overflow",
            Self::PlateAdded { .. } => "scene:plate_added",
            Self::PlateRemoved { .. } => "scene:plate_removed",
            Self::ActivePlateChanged { .. } => "scene:active_plate_changed",
            Self::ObjectOverridesChanged { .. } => "scene:object_overrides_changed",
            Self::ProjectOverridesChanged { .. } => "scene:project_overrides_changed",
            Self::PlateMetadataChanged { .. } => "scene:plate_metadata_changed",
            Self::MaterialBindingChanged { .. } => "scene:material_binding_changed",
            Self::ProjectSaved { .. } => "project:saved",
            Self::ProjectLoaded { .. } => "project:loaded",
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
    /// No plate with that id (PR-5-2).
    UnknownPlate(PlateId),
    /// Tried to remove the only remaining plate (PR-5-2).
    LastPlate,
    /// `move_object` was called with `from_plate == to_plate`
    /// (PR-5-11). Caller should check the source/dest are distinct.
    SamePlate(PlateId),
    /// Plate metadata validation rejected the new value (PR-5-5).
    /// `message` carries the validator's explanation suitable for
    /// surfacing as a toast.
    InvalidPlateMetadata {
        plate_id: PlateId,
        message: String,
    },
}

impl std::fmt::Display for SceneOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownObject(id) => write!(f, "no scene object with id {}", id.0),
            Self::UnknownMesh(id) => write!(f, "no mesh with id {}", id.0),
            Self::UnknownPlate(id) => write!(f, "no plate with id {}", id.0),
            Self::LastPlate => write!(f, "cannot remove the last plate"),
            Self::SamePlate(id) => {
                write!(f, "from_plate == to_plate ({}); pick a different target", id.0)
            }
            Self::InvalidPlateMetadata { plate_id, message } => {
                write!(f, "plate {}: {}", plate_id.0, message)
            }
        }
    }
}

/// Result of a [`SceneState::move_object`] call (PR-5-11). The
/// frontend reads `repositioned` to surface a toast when the
/// target's geometry forced the object away from its original
/// world position.
#[derive(Debug, Clone, Serialize)]
pub struct MoveReport {
    pub object_id: ObjectId,
    pub new_position: [f32; 3],
    /// `Some(reason)` when the original world-space position didn't
    /// fit on the target plate and the object was re-anchored.
    pub repositioned: Option<RepositionReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind")]
pub enum RepositionReason {
    /// The object's world-space bounding box landed outside the
    /// target plate's build volume in XY.
    OutOfBounds,
    /// The object's world-space bounding box intersects one of
    /// the target's exclusion zones.
    OnExclusionZone,
    /// The object's world-space minimum Z is below the target
    /// plate's bed surface.
    BelowBedSurface,
}

impl std::error::Error for SceneOpError {}
