//! Scene events — the diff payloads the renderer subscribes to.
//!
//! `Project`'s mutation methods (`select`, `translate`, etc.) are
//! *pure*: they take `&mut self` and return a `Vec<SceneEvent>`. The
//! Tauri wrapper in `commands.rs` takes that list and emits each
//! event via `Window::emit`. Tests bypass the Tauri layer and
//! inspect the returned events directly — no mock framework needed.
//!
//! Event names follow `scene:<noun>_<verb>` (e.g.
//! `scene:object_updated`). The frontend's `eventBridge.ts`
//! matches on these to update the local mirror.

use super::bed::{BedMesh, OutOfBoundsReason};
use super::state::{MeshHeader, MeshId, ObjectId, SceneObject};
use crate::core::project::model::PlateId;
use serde::Serialize;

/// One diff payload the renderer applies to its local mirror.
///
/// **Variant convention:** Every variant uses struct-shape fields
/// (not tuple shape) so consumers can pattern match by name and so
/// new fields can land without re-rolling the wire shape. Every
/// plate-scoped variant carries `plate_id: PlateId` as the first
/// field so the frontend mirror can route the event to the right
/// per-plate cache.
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
    MeshLoaded { mesh: MeshHeader },

    // ---- Per-plate scene-graph deltas -------------------------------
    ObjectAdded {
        plate_id: PlateId,
        object: SceneObject,
    },
    /// Full updated object — simpler than diff compression for MVP.
    /// Per-field diffs can replace this later if the 5 ms p99
    /// budget gets tight.
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
    /// Auto-arrange on a plate could not fit every visible object.
    /// Non-blocking; the placed objects still moved. UI flags the
    /// listed ids in the outliner so the user can resize / remove /
    /// move to a different plate.
    AutoArrangeOverflow {
        plate_id: PlateId,
        un_placed: Vec<ObjectId>,
    },
    /// A new plate was added. The frontend mirror
    /// reads `plate_id` to track it; subsequent events on this
    /// plate carry the same id.
    PlateAdded { plate_id: PlateId },
    /// A plate was removed. Pairs with
    /// `ActivePlateChanged` when the removed plate was the active
    /// one (the rebalanced active plate's id ships separately).
    PlateRemoved { plate_id: PlateId },
    /// The active plate changed. Emitted on explicit
    /// switches and on remove-of-active rebalancing.
    ActivePlateChanged { plate_id: PlateId },
    /// One or more cascade overrides on a specific object changed
    ///. The frontend re-runs cascade resolution to refresh
    /// the panel — the event carries no value payload because the
    /// resolver re-reads the override map directly.
    ObjectOverridesChanged {
        plate_id: PlateId,
        object_id: ObjectId,
    },
    /// One or more project-tier overrides on a plate changed
    ///. Same shape as `ObjectOverridesChanged` minus the
    /// object id — the cascade re-resolves with the updated plate
    /// override map.
    ProjectOverridesChanged { plate_id: PlateId },
    /// One or more **user-tier** (project-wide) overrides changed.
    /// Project-scoped, so no plate id — the resolver re-reads
    /// `Project.user_overrides`. Drives the project-level plugin surface.
    UserOverridesChanged,
    /// A plate's metadata changed — cycle count, composition order,
    /// or name. The frontend re-reads the
    /// plate's metadata via the project snapshot to refresh the tab
    /// badge / inputs.
    PlateMetadataChanged { plate_id: PlateId },
    /// A plate's material → slot routing changed. The
    /// frontend re-reads `plate.material_to_slot` via the snapshot
    /// to refresh the slot binding panel.
    MaterialSlotChanged { plate_id: PlateId },
    /// A project was written to disk. `path` is the
    /// container the writer just produced. UI updates the window
    /// title + recent-files list.
    ProjectSaved { path: String },
    /// A project was loaded from disk — the in-memory
    /// `Project` state has been replaced wholesale. The frontend
    /// drops every cached plate / mesh / object and re-fetches via
    /// `scene_snapshot`.
    ProjectLoaded { path: String },
    /// A foreign (OrcaSlicer / Bambu Studio) project was imported via
    /// Open project. Emitted right after `ProjectLoaded` (so the scene
    /// re-syncs first); the report lets the UI tell the user what mapped
    /// and what was dropped.
    ProjectImported {
        path: String,
        report: crate::core::orca_import::ImportReport,
    },
    /// The in-memory project was replaced by an undo/redo step — the
    /// frontend resyncs wholesale (same as `ProjectLoaded`) but the
    /// history isn't reset and the project becomes dirty again.
    ProjectRestored,
}

impl SceneEvent {
    /// The `scene:*` event name the Tauri layer emits this payload
    /// under. Matches the frontend's `eventBridge.ts` switch
    /// statement.
    pub fn name(&self) -> &'static str {
        match self {
            Self::MeshLoaded { .. } => "scene:mesh_loaded",
            Self::ObjectAdded { .. } => "scene:object_added",
            Self::ObjectUpdated { .. } => "scene:object_updated",
            Self::ObjectRemoved { .. } => "scene:object_removed",
            Self::SelectionChanged { .. } => "scene:selection_changed",
            Self::BedChanged { .. } => "scene:bed_changed",
            Self::ObjectOutOfBounds { .. } => "scene:object_out_of_bounds",
            Self::AutoArrangeOverflow { .. } => "scene:auto_arrange_overflow",
            Self::PlateAdded { .. } => "scene:plate_added",
            Self::PlateRemoved { .. } => "scene:plate_removed",
            Self::ActivePlateChanged { .. } => "scene:active_plate_changed",
            Self::ObjectOverridesChanged { .. } => "scene:object_overrides_changed",
            Self::ProjectOverridesChanged { .. } => "scene:project_overrides_changed",
            Self::UserOverridesChanged => "scene:user_overrides_changed",
            Self::PlateMetadataChanged { .. } => "scene:plate_metadata_changed",
            Self::MaterialSlotChanged { .. } => "scene:material_slot_changed",
            Self::ProjectSaved { .. } => "project:saved",
            Self::ProjectLoaded { .. } => "project:loaded",
            Self::ProjectImported { .. } => "project:imported",
            Self::ProjectRestored => "project:restored",
        }
    }

    /// How this event moves the project's dirty (unsaved-edits) state.
    /// Mirrors the frontend's `editEvents.ts` classification: content
    /// edits dirty the project; save/load/import return it to a clean
    /// baseline; selection, navigation, empty-plate structure, and
    /// warnings are neutral. The single source of truth for "is this an
    /// edit" — `DirtyTracker` consumes it in `emit_all`.
    pub fn dirty_effect(&self) -> DirtyEffect {
        match self {
            Self::ObjectAdded { .. }
            | Self::ObjectUpdated { .. }
            | Self::ObjectRemoved { .. }
            | Self::BedChanged { .. }
            | Self::ObjectOverridesChanged { .. }
            | Self::ProjectOverridesChanged { .. }
            | Self::UserOverridesChanged
            | Self::PlateMetadataChanged { .. }
            | Self::MaterialSlotChanged { .. }
            // Restoring an undo/redo step changes the live state → dirty,
            // but `history::track` excludes it from recording (it's a
            // navigation, not a new edit).
            | Self::ProjectRestored => DirtyEffect::Dirties,

            Self::ProjectSaved { .. }
            | Self::ProjectLoaded { .. }
            | Self::ProjectImported { .. } => DirtyEffect::Cleans,

            Self::MeshLoaded { .. }
            | Self::SelectionChanged { .. }
            | Self::ObjectOutOfBounds { .. }
            | Self::AutoArrangeOverflow { .. }
            | Self::PlateAdded { .. }
            | Self::PlateRemoved { .. }
            | Self::ActivePlateChanged { .. } => DirtyEffect::Neutral,
        }
    }
}

/// What an emitted [`SceneEvent`] does to the project's dirty state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyEffect {
    /// A content edit — the project now has unsaved changes.
    Dirties,
    /// Save / load / import — the project matches its on-disk form.
    Cleans,
    /// Selection, navigation, warnings — no effect on dirtiness.
    Neutral,
}

#[cfg(test)]
mod dirty_effect_tests {
    use super::*;
    use crate::core::project::model::PlateId;

    const P: PlateId = PlateId(1);

    #[test]
    fn content_edits_dirty_the_project() {
        for e in [
            SceneEvent::UserOverridesChanged,
            SceneEvent::ProjectOverridesChanged { plate_id: P },
            SceneEvent::MaterialSlotChanged { plate_id: P },
            SceneEvent::PlateMetadataChanged { plate_id: P },
            SceneEvent::BedChanged {
                plate_id: P,
                bed: None,
            },
        ] {
            assert_eq!(e.dirty_effect(), DirtyEffect::Dirties, "{e:?}");
        }
    }

    #[test]
    fn save_load_import_clean_the_project() {
        for e in [
            SceneEvent::ProjectSaved { path: String::new() },
            SceneEvent::ProjectLoaded { path: String::new() },
        ] {
            assert_eq!(e.dirty_effect(), DirtyEffect::Cleans, "{e:?}");
        }
    }

    #[test]
    fn selection_navigation_and_warnings_are_neutral() {
        for e in [
            SceneEvent::SelectionChanged {
                plate_id: P,
                selected: vec![],
            },
            SceneEvent::ActivePlateChanged { plate_id: P },
            SceneEvent::PlateAdded { plate_id: P },
            SceneEvent::PlateRemoved { plate_id: P },
            SceneEvent::AutoArrangeOverflow {
                plate_id: P,
                un_placed: vec![],
            },
        ] {
            assert_eq!(e.dirty_effect(), DirtyEffect::Neutral, "{e:?}");
        }
    }
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
    /// No plate with that id.
    UnknownPlate(PlateId),
    /// Tried to remove the only remaining plate.
    LastPlate,
    /// A move-between-plates op was called with `from_plate == to_plate`
    ///. Caller should check the source/dest are distinct.
    SamePlate(PlateId),
    /// Plate metadata validation rejected the new value.
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
                write!(
                    f,
                    "from_plate == to_plate ({}); pick a different target",
                    id.0
                )
            }
            Self::InvalidPlateMetadata { plate_id, message } => {
                write!(f, "plate {}: {}", plate_id.0, message)
            }
        }
    }
}

/// Outcome of a [`Project::rebind_plate_printer`] call: the from/to
/// printer identities and the new build plate, so the UI can update its
/// binding display.
#[derive(Debug, Clone, Serialize)]
pub struct PrinterChangeReport {
    pub plate_id: PlateId,
    /// Identity of the printer this plate was bound to before the
    /// change, or `None` if the plate was unbound.
    pub previous_printer: Option<String>,
    pub new_printer: String,
    pub new_build_plate: String,
}

impl std::error::Error for SceneOpError {}
