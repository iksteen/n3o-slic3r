//! Project + Plate types (PR-5-1).
//!
//! [`Project`] is the root state Phase 5 introduces — a list of
//! [`Plate`]s plus project-wide state (cascade handle, user-tier
//! overrides, file metadata, source path). Each [`Plate`] owns its
//! own printer binding, project-tier overrides, material bindings,
//! plate metadata, and per-plate scene state.
//!
//! PR-5-2 refactors the existing single-global `SceneState` into
//! the per-plate [`PlateSceneState`] declared here. PR-5-8 ships
//! the `.3mf` save/load that round-trips this whole shape.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::binding::{MaterialBinding, PrinterBinding};
use super::metadata::PlateMetadata;
use crate::core::cascade::commands::CascadeHandle;

/// Opaque 1-based plate id. Stable across the plate list —
/// reordering doesn't change the id, only the position. Survives
/// save/load via [`PR-5-8`](super#).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlateId(pub u32);

/// Project-root state. Replaces the implicit single-plate worldview
/// the Phase 2-4 code carried. PR-5-2's `SceneState` refactor wraps
/// the per-plate [`PlateSceneState`]s under here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Stable per-project identifier; baked at construction so the
    /// autosave path (PR-5-10) can use it to dedupe across
    /// concurrent app instances editing different projects.
    pub uuid: Uuid,

    /// All plates in the project, in declaration order. The first
    /// plate's id is `PlateId(1)`. Reordering this list does not
    /// renumber ids; reordering is purely positional.
    pub plates: Vec<Plate>,

    /// Index into `plates` for the currently-active plate. A
    /// default-constructed `Project` has one plate and
    /// `active_plate = 0`. Always points at a valid index — see
    /// [`Project::remove_plate`] for the invariant maintenance.
    pub active_plate: usize,

    /// Cascade handle (from PR-1-9's `CascadeRegistry`). One
    /// cascade per project — per-plate overrides layer on top.
    /// Phase 5 ships single-cascade-per-project; future work can
    /// introduce per-plate cascades if a workflow demands it.
    pub cascade_handle: CascadeHandle,

    /// User-tier overrides (FR-CAS-3). Apply across every plate.
    /// Project-tier overrides live on each [`Plate`].
    #[serde(default)]
    pub user_overrides: HashMap<String, String>,

    /// File-level 3MF metadata: Title, Designer, License, …
    /// Round-trips through PR-5-8's save/load. Keys mirror the
    /// `<metadata name="…">` element format from the 3MF Core
    /// spec.
    #[serde(default)]
    pub file_metadata: BTreeMap<String, String>,

    /// Filesystem path the project came from / saves to.
    /// `None` for in-memory projects that haven't been saved
    /// yet (the PR-5-10 autosave still runs for those — the
    /// autosave path is derived from `uuid`, not `source_path`).
    #[serde(default)]
    pub source_path: Option<PathBuf>,
}

/// A single plate: bound printer + build plate, project-tier
/// overrides scoped to this plate, material bindings, plate
/// metadata, and the plate's scene contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plate {
    pub id: PlateId,

    /// Display name for the tab strip. Defaults to "Plate N";
    /// user-renamable via PR-5-3.
    pub name: String,

    pub printer: PrinterBinding,

    /// Project-tier overrides scoped to this plate. The cascade
    /// resolves each plate against the union of `Project.user_overrides`
    /// (which apply everywhere) and this map (which applies only
    /// to this plate). PR-4-9's per-object overrides layer above
    /// both inside the plate's scene state.
    #[serde(default)]
    pub project_overrides: HashMap<String, String>,

    /// Per-(plate, printer) model-material → physical-slot
    /// bindings. Empty for fresh plates; PR-5-6's auto-bind
    /// pre-fills based on loaded filaments at first printer
    /// assignment.
    #[serde(default)]
    pub material_bindings: Vec<MaterialBinding>,

    pub metadata: PlateMetadata,

    /// The plate's scene contents. PR-5-2 fills this struct out;
    /// PR-5-1 ships it as an empty placeholder so [`Project`]
    /// compiles + serializes.
    #[serde(default)]
    pub scene: PlateSceneState,
}

/// Per-plate scene state — objects, mesh refs, selection, bed mesh,
/// gizmo mode, camera. **Stub in PR-5-1**; PR-5-2 hoists the
/// existing single-global `SceneState` fields into here and
/// rewires the Tauri command surface to address per-plate.
///
/// Implementing it as an empty struct now lets [`Plate`] declare
/// the field with `#[serde(default)]` so PR-5-1's round-trip
/// tests pass + PR-5-8's save/load is unblocked. Field additions
/// in PR-5-2 only need to ensure `Default` keeps producing a
/// usable empty plate.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlateSceneState {
    // PR-5-2 fills:
    //   pub objects: HashMap<ObjectId, SceneObject>,
    //   pub meshes: HashMap<MeshId, MeshHeader>,
    //   pub selection: Vec<ObjectId>,
    //   pub gizmo: Option<GizmoMode>,
    //   pub camera: Camera,
    //   pub bed: Option<BedMesh>,
    //   pub exclusion_zones: Vec<ExclusionZone>,
    //   pub object_overrides: HashMap<ObjectId, HashMap<String, String>>,
    // Plus the matching invariant-maintenance methods.
}

impl Project {
    /// Build a single-plate default project bound to the given
    /// printer + cascade. Matches the Phase 0-4 single-plate
    /// worldview as the starting state — multi-plate is purely
    /// additive.
    pub fn new(cascade_handle: CascadeHandle, printer: PrinterBinding) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            plates: vec![Plate::new(PlateId(1), printer, 1)],
            active_plate: 0,
            cascade_handle,
            user_overrides: HashMap::new(),
            file_metadata: BTreeMap::new(),
            source_path: None,
        }
    }

    /// Append a new plate bound to `printer`. The new plate's id
    /// is `max(existing ids) + 1` (stable across the list);
    /// `composition_order` defaults to the plate's position in
    /// the list. Returns the new id.
    pub fn add_plate(&mut self, printer: PrinterBinding) -> PlateId {
        let next_id = self
            .plates
            .iter()
            .map(|p| p.id.0)
            .max()
            .map(|m| m + 1)
            .unwrap_or(1);
        let position = (self.plates.len() + 1) as u32;
        self.plates.push(Plate::new(PlateId(next_id), printer, position));
        PlateId(next_id)
    }

    /// Drop a plate by id. Errors when:
    ///   - The plate id isn't in the list.
    ///   - It's the only plate (FR-MP-1: 1-4 plates; a project
    ///     must always have at least one).
    ///
    /// On success, repacks `composition_order` so the remaining
    /// plates form a dense `[1..N]` sequence + clamps
    /// `active_plate` if the removed plate was the active one.
    pub fn remove_plate(&mut self, id: PlateId) -> Result<(), ProjectMutError> {
        if self.plates.len() <= 1 {
            return Err(ProjectMutError::LastPlate);
        }
        let idx = self
            .plates
            .iter()
            .position(|p| p.id == id)
            .ok_or(ProjectMutError::NoSuchPlate(id))?;
        self.plates.remove(idx);
        if self.active_plate >= self.plates.len() {
            self.active_plate = self.plates.len() - 1;
        }
        // Renumber composition_order so the remaining plates form
        // [1..N] without gaps. Preserves relative ordering.
        let mut order_pairs: Vec<(usize, u32)> = self
            .plates
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.metadata.composition_order))
            .collect();
        order_pairs.sort_by_key(|&(_, order)| order);
        for (new_pos, (i, _)) in order_pairs.into_iter().enumerate() {
            self.plates[i].metadata.composition_order = (new_pos + 1) as u32;
        }
        Ok(())
    }

    /// Convenience accessor for the active plate. Panics if the
    /// project is empty (shouldn't happen — `Project` invariants
    /// guarantee ≥ 1 plate).
    pub fn active_plate(&self) -> &Plate {
        &self.plates[self.active_plate]
    }

    /// Mutable accessor. Same panic invariant.
    pub fn active_plate_mut(&mut self) -> &mut Plate {
        &mut self.plates[self.active_plate]
    }

    /// Find a plate by id.
    pub fn plate(&self, id: PlateId) -> Option<&Plate> {
        self.plates.iter().find(|p| p.id == id)
    }

    pub fn plate_mut(&mut self, id: PlateId) -> Option<&mut Plate> {
        self.plates.iter_mut().find(|p| p.id == id)
    }
}

impl Plate {
    /// Construct a plate at the given 1-based position with the
    /// default name. Used by [`Project::new`] and
    /// [`Project::add_plate`].
    pub fn new(id: PlateId, printer: PrinterBinding, position: u32) -> Self {
        Self {
            id,
            name: Self::default_name(position),
            printer,
            project_overrides: HashMap::new(),
            material_bindings: Vec::new(),
            metadata: PlateMetadata::at_position(position),
            scene: PlateSceneState::default(),
        }
    }

    /// Default name for a plate at the given 1-based position
    /// — "Plate 1", "Plate 2", …. Users can rename via PR-5-3.
    pub fn default_name(position: u32) -> String {
        format!("Plate {position}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectMutError {
    LastPlate,
    NoSuchPlate(PlateId),
}

impl std::fmt::Display for ProjectMutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LastPlate => write!(f, "cannot remove the last plate (a project must have ≥ 1 plate)"),
            Self::NoSuchPlate(id) => write!(f, "no plate with id {}", id.0),
        }
    }
}

impl std::error::Error for ProjectMutError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn a1_mini() -> PrinterBinding {
        PrinterBinding {
            printer_identity: "bambu-a1-mini".into(),
            build_plate_identity: "Textured PEI".into(),
        }
    }

    fn snapmaker_u1() -> PrinterBinding {
        PrinterBinding {
            printer_identity: "snapmaker-u1".into(),
            build_plate_identity: "Textured PEI".into(),
        }
    }

    #[test]
    fn new_project_is_single_plate_active_zero() {
        let p = Project::new(7, a1_mini());
        assert_eq!(p.plates.len(), 1);
        assert_eq!(p.plates[0].id, PlateId(1));
        assert_eq!(p.plates[0].name, "Plate 1");
        assert_eq!(p.active_plate, 0);
        assert_eq!(p.cascade_handle, 7);
    }

    #[test]
    fn add_plate_assigns_monotonic_ids_and_default_name() {
        let mut p = Project::new(1, a1_mini());
        let id_a = p.add_plate(snapmaker_u1());
        let id_b = p.add_plate(snapmaker_u1());
        assert_eq!(id_a, PlateId(2));
        assert_eq!(id_b, PlateId(3));
        assert_eq!(p.plates.len(), 3);
        assert_eq!(p.plates[1].name, "Plate 2");
        assert_eq!(p.plates[2].name, "Plate 3");
        assert_eq!(p.plates[2].metadata.composition_order, 3);
    }

    #[test]
    fn remove_plate_errors_on_last_plate() {
        let mut p = Project::new(1, a1_mini());
        let err = p.remove_plate(PlateId(1)).unwrap_err();
        assert_eq!(err, ProjectMutError::LastPlate);
        assert_eq!(p.plates.len(), 1);
    }

    #[test]
    fn remove_plate_errors_on_unknown_id() {
        let mut p = Project::new(1, a1_mini());
        p.add_plate(snapmaker_u1());
        let err = p.remove_plate(PlateId(99)).unwrap_err();
        assert_eq!(err, ProjectMutError::NoSuchPlate(PlateId(99)));
    }

    #[test]
    fn remove_plate_clamps_active_plate_when_active_is_removed() {
        let mut p = Project::new(1, a1_mini());
        p.add_plate(snapmaker_u1());
        p.add_plate(snapmaker_u1());
        p.active_plate = 2;
        p.remove_plate(PlateId(3)).unwrap();
        assert_eq!(p.plates.len(), 2);
        assert_eq!(p.active_plate, 1, "active_plate clamped to last valid index");
    }

    #[test]
    fn remove_plate_renumbers_composition_order_to_dense_sequence() {
        let mut p = Project::new(1, a1_mini());
        p.add_plate(snapmaker_u1());
        p.add_plate(snapmaker_u1());
        // Plates' default composition_orders: 1, 2, 3.
        p.remove_plate(PlateId(2)).unwrap();
        // After removing the middle plate, the remaining plates
        // should renumber to [1, 2] without gaps.
        assert_eq!(p.plates[0].metadata.composition_order, 1);
        assert_eq!(p.plates[1].metadata.composition_order, 2);
    }

    #[test]
    fn project_serde_round_trips() {
        let mut p = Project::new(42, a1_mini());
        p.add_plate(snapmaker_u1());
        p.plates[1].project_overrides.insert("layer_height".into(), "0.12".into());
        p.plates[1].material_bindings.push(MaterialBinding {
            model_material: 1,
            physical_slot: 2,
            filament_identity: "Generic PLA".into(),
        });
        p.plates[1].metadata.cycle_count = 3;
        p.user_overrides.insert("travel_speed".into(), "300".into());
        p.file_metadata.insert("Title".into(), "Test Project".into());

        let json = serde_json::to_string(&p).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.uuid, p.uuid);
        assert_eq!(parsed.cascade_handle, 42);
        assert_eq!(parsed.plates.len(), 2);
        assert_eq!(parsed.plates[1].project_overrides.get("layer_height").map(|s| s.as_str()), Some("0.12"));
        assert_eq!(parsed.plates[1].material_bindings.len(), 1);
        assert_eq!(parsed.plates[1].metadata.cycle_count, 3);
        assert_eq!(parsed.user_overrides.get("travel_speed").map(|s| s.as_str()), Some("300"));
        assert_eq!(parsed.file_metadata.get("Title").map(|s| s.as_str()), Some("Test Project"));
    }

    #[test]
    fn plate_id_serializes_as_bare_integer() {
        let id = PlateId(5);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "5", "PlateId uses serde(transparent) — bare int on the wire");
        let parsed: PlateId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }
}
