//! Project + Plate types.
//!
//! [`Project`] is the root authoritative state. Tauri manages
//! `Mutex<Project>` (one per app instance). The renderer is a
//! read-only consumer that mirrors per-plate state via emitted
//! events.
//!
//! Project owns:
//!   - the plate list + active plate (project navigation)
//!   - cascade handle + project-wide override tier (cascade
//!     resolution input)
//!   - file metadata + source path (save/load — PR-5-8)
//!   - scene-wide mesh storage + ID allocators (so cross-plate
//!     references survive PR-5-11 move-between-plates without
//!     a copy + ids stay unique across plates)
//!
//! Each [`Plate`] owns:
//!   - its printer binding + build plate (cascade context input)
//!   - its project-tier overrides (override-tier resolution)
//!   - its material → slot bindings (FR-MP-8)
//!   - its metadata (cycle count, composition order)
//!   - its scene contents
//!     (`core::scene::state::PlateSceneState` — objects, selection,
//!     gizmo, camera, bed, exclusion zones, per-object overrides)
//!
//! Mutation methods live in [`super::mutation`] to keep this file
//! focused on the type shape. Project-level mutations
//! (`add_plate`, `remove_plate`, `set_active_plate`) and scene
//! mutations (`translate_object`, `select`, …) both operate on
//! `&mut Project` via the same impl block.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::binding::{MaterialBinding, PrinterBinding};
use super::metadata::PlateMetadata;
use crate::core::cascade::commands::CascadeHandle;
use crate::core::scene::state::{Mesh, MeshId, PlateSceneState};

/// Opaque 1-based plate id. Stable across the plate list —
/// reordering doesn't change the id, only the position. Survives
/// save/load via [`PR-5-8`](super#).
///
/// Public Tauri commands address plates by `PlateId`, not by index
/// — the frontend can hold a stable reference even when plates are
/// reordered or removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlateId(pub u32);

/// Project-root state. Tauri manages `Arc<Mutex<Project>>` (one
/// per app instance, wrapped in an Arc so the autosave worker
/// can hold a handle without going through Tauri state lookup);
/// every Tauri command takes `State<Arc<Mutex<Project>>>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Stable per-project identifier; baked at construction so the
    /// autosave path (PR-5-10) can use it to dedupe across
    /// concurrent app instances editing different projects.
    pub uuid: Uuid,

    /// All plates in the project, in declaration order. The first
    /// default plate's id is `PlateId(1)`. Reordering this list
    /// does not renumber ids; reordering is purely positional.
    pub plates: Vec<Plate>,

    /// Index into `plates` for the currently-active plate. Always
    /// points at a valid index — see
    /// [`Self::remove_plate`](super::mutation) for the
    /// invariant maintenance. Internal; the public command surface
    /// addresses plates by `PlateId`.
    pub active_plate: usize,

    /// Currently-loaded cascade handle (from PR-1-9's
    /// `CascadeRegistry`). `None` at app startup before the user
    /// loads a cascade; bound to `Some(handle)` after the
    /// `cascade_load` Tauri command returns its id.
    ///
    /// One cascade per project — per-plate overrides layer on top.
    /// Phase 5 ships single-cascade-per-project; future work can
    /// introduce per-plate cascades if a workflow demands it.
    #[serde(default)]
    pub cascade_handle: Option<CascadeHandle>,

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

    /// Scene-wide mesh storage. Per-plate object references
    /// (`Plate.scene.objects[*].mesh`) resolve through this map.
    /// Living scene-wide means PR-5-11's move-between-plates op
    /// doesn't have to copy mesh buffers across plates.
    #[serde(default)]
    pub meshes: HashMap<MeshId, Mesh>,

    /// Primitive mesh cache (PR-2-7). Each (kind, params) tuple
    /// resolves to one MeshId so re-instancing the same procedural
    /// primitive — across plates as well as within a plate — yields
    /// multiple SceneObjects sharing geometry. Linear scan is fine
    /// (the cache stays small: a handful of distinct shapes per
    /// session).
    #[serde(default, skip)]
    pub(crate) primitive_cache: Vec<(
        crate::core::scene::primitives::PrimitiveKind,
        crate::core::scene::primitives::PrimitiveParams,
        MeshId,
    )>,

    /// Monotonic mesh-id allocator. Never reused even after a
    /// mesh is freed. Scene-wide (not per-plate) so cross-plate
    /// references stay unambiguous.
    pub(crate) next_mesh_id: u64,

    /// Monotonic object-id allocator. Same semantics as
    /// `next_mesh_id`.
    pub(crate) next_object_id: u64,
}

/// A single plate. Composes plate-level metadata + bindings with
/// the per-plate scene contents (`PlateSceneState`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plate {
    pub id: PlateId,

    /// Display name for the tab strip. Defaults to "Plate N";
    /// user-renamable via PR-5-3.
    pub name: String,

    /// Bound printer + build plate. `None` for a freshly-added
    /// plate that hasn't been assigned a printer yet — the
    /// `+` button in PR-5-3's tab strip creates a plate without
    /// a printer; the user picks one via PR-5-4's picker. The
    /// cascade context for an unbound plate is undefined; the
    /// slice command refuses to run until the binding is set.
    #[serde(default)]
    pub printer: Option<PrinterBinding>,

    /// Project-tier overrides scoped to this plate. The cascade
    /// resolves each plate against the union of
    /// `Project.user_overrides` (applies everywhere) and this map
    /// (applies only to this plate). PR-4-9's per-object overrides
    /// layer above both inside the plate's scene state.
    #[serde(default)]
    pub project_overrides: HashMap<String, String>,

    /// Per-(plate, printer) model-material → physical-slot
    /// bindings. Empty for fresh plates; PR-5-6's auto-bind
    /// pre-fills based on loaded filaments at first printer
    /// assignment.
    #[serde(default)]
    pub material_bindings: Vec<MaterialBinding>,

    pub metadata: PlateMetadata,

    /// The plate's scene contents. Real type lives in
    /// `core::scene::state::PlateSceneState` (PR-5-2); `Plate`
    /// composes it so PR-5-8's project `.3mf` save/load
    /// round-trips the full per-plate scene alongside the plate
    /// metadata.
    #[serde(default)]
    pub scene: PlateSceneState,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            uuid: Uuid::new_v4(),
            plates: vec![Plate::new(PlateId(1), 1)],
            active_plate: 0,
            cascade_handle: None,
            user_overrides: HashMap::new(),
            file_metadata: BTreeMap::new(),
            source_path: None,
            meshes: HashMap::new(),
            primitive_cache: Vec::new(),
            next_mesh_id: 0,
            next_object_id: 0,
        }
    }
}

impl Project {
    /// Empty single-plate project. Same as [`Project::default`];
    /// kept as a named constructor for callsite readability.
    pub fn new() -> Self {
        Self::default()
    }

    /// The currently-active plate. Panics if the project is empty
    /// (invariant: `plates.len() ≥ 1` always).
    pub fn active_plate(&self) -> &Plate {
        &self.plates[self.active_plate]
    }

    /// Mutable view of the active plate.
    pub fn active_plate_mut(&mut self) -> &mut Plate {
        &mut self.plates[self.active_plate]
    }

    /// Look up a plate by id.
    pub fn plate(&self, id: PlateId) -> Option<&Plate> {
        self.plates.iter().find(|p| p.id == id)
    }

    /// Mutable look-up.
    pub fn plate_mut(&mut self, id: PlateId) -> Option<&mut Plate> {
        self.plates.iter_mut().find(|p| p.id == id)
    }

    /// Resolve a `PlateId` to its current index in `plates`.
    /// Returns `None` when the id isn't in the project. Used
    /// internally by mutation methods that need indexed access
    /// (the borrow checker needs the index, not a borrowed Plate,
    /// to mutate sibling plates).
    pub(crate) fn plate_index(&self, id: PlateId) -> Option<usize> {
        self.plates.iter().position(|p| p.id == id)
    }
}

impl Plate {
    /// Validate this plate's material bindings against the bound
    /// printer's slot count and the model materials its objects
    /// reference (FR-MP-8 / PR-5-6).
    ///
    /// Returns an empty `Vec` when the plate is ready to slice.
    /// A non-empty `Vec` lists every problem; the pre-slice gate
    /// surfaces these as `slice_blocker` errors.
    ///
    /// `slot_count` is caller-supplied — the plate holds the
    /// printer *identity* via [`PrinterBinding`], not the resolved
    /// profile. The caller (Tauri command layer / slice
    /// orchestrator) loads the profile and passes its slot count
    /// in.
    pub fn validate_material_bindings(&self, slot_count: u8) -> Vec<super::BindingIssue> {
        use super::BindingIssue;
        let mut issues = Vec::new();

        // Pass 1: bindings' own field validity + slot-range check
        // + duplicate detection.
        let mut seen_materials = std::collections::HashSet::new();
        for b in &self.material_bindings {
            if let Err(msg) = b.validate() {
                issues.push(BindingIssue::InvalidBinding {
                    model_material: b.model_material,
                    message: msg,
                });
                continue;
            }
            if !seen_materials.insert(b.model_material) {
                issues.push(BindingIssue::DuplicateMaterial {
                    model_material: b.model_material,
                });
                continue;
            }
            if b.physical_slot > slot_count {
                issues.push(BindingIssue::SlotOutOfRange {
                    model_material: b.model_material,
                    physical_slot: b.physical_slot,
                    slot_count,
                });
            }
        }

        // Pass 2: every model material referenced by an object on
        // this plate must have a binding entry.
        let bound: std::collections::HashSet<u8> = self
            .material_bindings
            .iter()
            .map(|b| b.model_material)
            .collect();
        let mut referenced: std::collections::BTreeSet<u8> =
            std::collections::BTreeSet::new();
        for obj in self.scene.objects.values() {
            if let Some(mat) = obj.extruder_id {
                if mat >= 1 {
                    referenced.insert(mat);
                }
            }
        }
        for mat in referenced {
            if !bound.contains(&mat) {
                issues.push(BindingIssue::UnboundMaterial { model_material: mat });
            }
        }

        issues
    }

    /// Construct an empty plate (no printer assigned) at the
    /// given 1-based position with the default name.
    pub fn new(id: PlateId, position: u32) -> Self {
        Self {
            id,
            name: Self::default_name(position),
            printer: None,
            project_overrides: HashMap::new(),
            material_bindings: Vec::new(),
            metadata: PlateMetadata::at_position(position),
            scene: PlateSceneState::default(),
        }
    }

    /// Construct a plate already bound to the given printer.
    pub fn with_printer(id: PlateId, printer: PrinterBinding, position: u32) -> Self {
        Self {
            printer: Some(printer),
            ..Self::new(id, position)
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
            Self::LastPlate => write!(
                f,
                "cannot remove the last plate (a project must have ≥ 1 plate)",
            ),
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

    #[test]
    fn default_project_has_one_empty_plate_no_cascade() {
        let p = Project::default();
        assert_eq!(p.plates.len(), 1);
        assert_eq!(p.plates[0].id, PlateId(1));
        assert_eq!(p.plates[0].name, "Plate 1");
        assert_eq!(p.plates[0].printer, None, "empty plate has no printer");
        assert_eq!(p.active_plate, 0);
        assert_eq!(p.cascade_handle, None, "no cascade loaded at startup");
        assert!(p.meshes.is_empty());
        assert_eq!(p.next_mesh_id, 0);
        assert_eq!(p.next_object_id, 0);
    }

    #[test]
    fn plate_with_printer_constructor_attaches_printer() {
        let p = Plate::with_printer(PlateId(2), a1_mini(), 2);
        assert_eq!(p.id, PlateId(2));
        assert_eq!(p.name, "Plate 2");
        assert_eq!(p.printer, Some(a1_mini()));
    }

    #[test]
    fn project_serde_round_trips() {
        let mut p = Project::default();
        p.cascade_handle = Some(42);
        p.plates[0].printer = Some(a1_mini());
        p.plates[0]
            .project_overrides
            .insert("layer_height".into(), "0.12".into());
        p.plates[0].material_bindings.push(MaterialBinding {
            model_material: 1,
            physical_slot: 2,
            filament_identity: "Generic PLA".into(),
        });
        p.user_overrides
            .insert("travel_speed".into(), "300".into());
        p.file_metadata
            .insert("Title".into(), "Test Project".into());

        let json = serde_json::to_string(&p).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.uuid, p.uuid);
        assert_eq!(parsed.cascade_handle, Some(42));
        assert_eq!(parsed.plates.len(), 1);
        assert_eq!(parsed.plates[0].printer, Some(a1_mini()));
        assert_eq!(
            parsed.plates[0]
                .project_overrides
                .get("layer_height")
                .map(|s| s.as_str()),
            Some("0.12"),
        );
        assert_eq!(parsed.plates[0].material_bindings.len(), 1);
        assert_eq!(
            parsed.user_overrides.get("travel_speed").map(|s| s.as_str()),
            Some("300"),
        );
        assert_eq!(
            parsed.file_metadata.get("Title").map(|s| s.as_str()),
            Some("Test Project"),
        );
    }

    #[test]
    fn plate_id_serializes_as_bare_integer() {
        let id = PlateId(5);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(
            json, "5",
            "PlateId uses serde(transparent) — bare int on the wire",
        );
        let parsed: PlateId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn plate_lookup_by_id() {
        let mut p = Project::default();
        let plate = Plate::with_printer(PlateId(7), a1_mini(), 2);
        p.plates.push(plate);
        assert_eq!(p.plate(PlateId(7)).map(|x| x.id), Some(PlateId(7)));
        assert!(p.plate(PlateId(99)).is_none());
        assert_eq!(p.plate_index(PlateId(7)), Some(1));
        assert_eq!(p.plate_index(PlateId(99)), None);
    }
}
