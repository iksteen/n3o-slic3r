//! Project + Plate types.
//!
//! [`Project`] is the **persisted** root state — pure serializable content.
//! It's wrapped in a [`super::session::Session`] (the Tauri-managed
//! `Mutex<Session>`) that pairs it with the runtime state (`source_path`,
//! selection, derived beds) so nothing ephemeral lives in the project. The
//! renderer is a read-only consumer that mirrors per-plate state via events.
//!
//! Project owns:
//!   - the plate list + active plate (project navigation)
//!   - cascade handle + project-wide override tier (cascade
//!     resolution input)
//!   - file metadata (save/load)
//!   - scene-wide mesh storage + ID allocators (so cross-plate
//!     references survive move-between-plates without a copy +
//!     ids stay unique across plates)
//!
//! Each [`Plate`] owns:
//!   - its printer binding + build plate (cascade context input)
//!   - its project-tier overrides (override-tier resolution)
//!   - its material → slot bindings (FR-MP-8)
//!   - its metadata (cycle count, composition order)
//!   - its scene contents
//!     (`core::scene::state::PlateSceneState` — objects + per-object
//!     overrides; selection + bed live in `SessionRuntime`)
//!
//! Mutation methods live in [`super::mutation`] to keep this file
//! focused on the type shape. Project-level mutations
//! (`add_plate`, `remove_plate`, `set_active_plate`) and scene
//! mutations (`set_object_transform`, `select`, …) both operate on
//! `&mut Project` via the same impl block.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::printer::PrinterInstance;
use crate::core::scene::state::{Mesh, MeshId, PlateSceneState};

/// Opaque 1-based plate id. Stable across the plate list —
/// reordering doesn't change the id, only the position. Survives
/// project save/load.
///
/// Public Tauri commands address plates by `PlateId`, not by index
/// — the frontend can hold a stable reference even when plates are
/// reordered or removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlateId(pub u32);

/// Project-root state. Tauri manages `Arc<Mutex<Session>>` (one
/// per app instance, wrapped in an Arc so the autosave worker
/// can hold a handle without going through Tauri state lookup);
/// every Tauri command takes `State<Arc<Mutex<Session>>>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Stable per-project identifier; baked at construction so the
    /// autosave path can use it to dedupe across
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

    /// User-tier overrides (FR-CAS-3). Apply across every plate.
    /// Project-tier overrides live on each [`Plate`].
    #[serde(default)]
    pub user_overrides: HashMap<String, String>,

    /// File-level 3MF metadata: Title, Designer, License, …
    /// Round-trips through save/load. Keys mirror the
    /// `<metadata name="…">` element format from the 3MF Core
    /// spec.
    #[serde(default)]
    pub file_metadata: BTreeMap<String, String>,

    /// Scene-wide mesh storage. Per-plate object references
    /// (`Plate.scene.objects[*].mesh`) resolve through this map.
    /// Living scene-wide means move-between-plates op
    /// doesn't have to copy mesh buffers across plates.
    #[serde(default)]
    pub meshes: HashMap<MeshId, Mesh>,

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
    /// user-renamable (tab strip dblclick).
    pub name: String,

    /// Project-tier overrides scoped to this plate. The cascade
    /// resolves each plate against the union of
    /// `Project.user_overrides` (applies everywhere) and this map
    /// (applies only to this plate). per-object overrides
    /// layer above both inside the plate's scene state.
    #[serde(default)]
    pub project_overrides: HashMap<String, String>,

    /// Names the [`PrinterInstance`] this plate slices against. The
    /// composer assembles the slice-time cascade from the instance's
    /// vendor printer/filament/process fragments + per-extruder
    /// nozzle.tomls + this plate's process overrides. `None` for an
    /// unbound plate (slice refuses with `UnboundPrinter`).
    ///
    /// Sole carrier of binding state — the vendor profile identity
    /// (e.g. `"bambu-lab-a1-mini"`) is derived on demand via
    /// `lookup_instance(id).vendor_profile_ref` rather than stored
    /// alongside.
    ///
    /// **Private on purpose**: set only via [`Plate::set_printer`]. The
    /// derived bed visualization follows this binding but lives in
    /// `SessionRuntime` (re-derived by `Session::reconcile`), not here.
    /// Read via [`Plate::printer_instance_id`].
    #[serde(default)]
    printer_instance_id: Option<String>,

    /// Per-plate routing from a model material index (the per-volume
    /// `extruder` metadata libslic3r consumes) to a specific feed slot
    /// on the bound `PrinterInstance`. Auto-populated when
    /// `register_object` lands a new material (first available slot,
    /// walked extruder-major), user-editable via the slot binding
    /// panel. Empty for plates with no material-tagged objects.
    ///
    /// Cleared on printer swap — slot refs are physical coordinates
    /// (`(extruder, slot)`) that don't survive a topology change.
    ///
    /// `BTreeMap` (not `HashMap`) so the wire form stays deterministic
    /// for save/load + diff displays.
    #[serde(default)]
    pub material_to_slot: std::collections::BTreeMap<u8, crate::core::printer::SlotRef>,

    /// The process/quality profile this plate slices against, as a
    /// bundled process-fragment slug (e.g. `"0.20mm-strength"`). `None`
    /// inherits the bound `PrinterInstance.quality_profile` — the
    /// instance's profile is the seed/default; selecting one per plate
    /// (or importing a project authored with a different preset) records
    /// it here so this plate resolves + slices against that process
    /// without touching the shared instance. The composer's effective
    /// process is `plate.quality_profile.unwrap_or(instance.quality_profile)`.
    #[serde(default)]
    pub quality_profile: Option<String>,

    /// The plate's scene contents. Real type lives in
    /// `core::scene::state::PlateSceneState`; `Plate` composes it
    /// so native `.n3o` save/load round-trips the full per-plate
    /// scene alongside the plate metadata.
    #[serde(default)]
    pub scene: PlateSceneState,
}

impl Default for Project {
    fn default() -> Self {
        // Pure empty project: a single unbound plate, no registry access. The
        // model never reaches for the printer registry — not even to bootstrap.
        // The command layer resolves the registry and calls
        // `with_preferred_printer` with the instance list to bind the user's
        // preferred/first printer.
        Self::with_preferred_printer(None, &[])
    }
}

/// Bind a `PrinterInstance` to `plate`: the instance from `instances` matching
/// `preferred` if any, otherwise the first one. Silent no-op when `instances` is
/// empty (bootstrap plate stays unbound → the empty-state onboarding takes over)
/// or neither is available.
fn bind_preferred_else_first_in_place(
    plate: &mut Plate,
    preferred: Option<&str>,
    instances: &[PrinterInstance],
) {
    let chosen = preferred
        .and_then(|p| instances.iter().find(|i| i.id == p))
        .or_else(|| instances.first());
    let Some(inst) = chosen else {
        return;
    };
    // Bind the id (persisted); the bed derives when the project is wrapped in
    // a `Session` (`Session::new` reconciles).
    plate.set_printer(Some(inst.id.clone()));
}

impl Project {
    /// Empty single-plate project. Same as [`Project::default`];
    /// kept as a named constructor for callsite readability.
    pub fn new() -> Self {
        Self::default()
    }

    /// Human-readable project title for sliced-output naming + the `.gcode.3mf`
    /// `Title` metadata: a loaded non-empty `Title`, else the project file's stem
    /// (`source_path` lives in `SessionRuntime`, so the caller passes it), else
    /// "Untitled".
    pub fn title(&self, source_path: Option<&std::path::Path>) -> String {
        self.file_metadata
            .get("Title")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                source_path
                    .and_then(|p| p.file_stem())
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "Untitled".to_owned())
    }

    /// Empty single-plate project whose bootstrap plate binds `preferred` if
    /// it's in `instances`, else the first of `instances` (else stays unbound).
    /// The command layer resolves the registry and passes the user's
    /// last-selected printer (config `[defaults]`) as `preferred`;
    /// [`Project::default`] passes `None` + an empty list (pure, unbound).
    pub fn with_preferred_printer(preferred: Option<&str>, instances: &[PrinterInstance]) -> Self {
        let mut plate = Plate::new(PlateId(1), 1);
        bind_preferred_else_first_in_place(&mut plate, preferred, instances);
        Self {
            uuid: Uuid::new_v4(),
            plates: vec![plate],
            active_plate: 0,
            user_overrides: HashMap::new(),
            file_metadata: BTreeMap::new(),
            meshes: HashMap::new(),
            next_mesh_id: 0,
            next_object_id: 0,
        }
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

    /// Whether any object on `plate` references a mesh that carries MMU
    /// color-painting. Meshes are scene-wide (owned by `Project`, not the
    /// plate), so this query lives here. Drives the toolchanger paint-remap
    /// at slice time and the painted-plate material binding on import.
    pub fn plate_has_painted_object(&self, plate: &Plate) -> bool {
        plate.scene.objects.values().any(|o| {
            self.meshes
                .get(&o.mesh)
                .is_some_and(|m| m.paint_colors.is_some())
        })
    }

    /// Every material the plate uses: each object's `extruder_id` assignment
    /// PLUS the filament states referenced by MMU paint on its meshes.
    ///
    /// The paint-derived half is the authoritative source for a face-painted
    /// material — no object's `extruder_id` names it, so any logic that derives
    /// "materials in use" from objects alone (rebind, orphan-binding cleanup)
    /// must consult this or the painted material silently disappears.
    pub fn materials_on_plate(&self, plate: &Plate) -> std::collections::BTreeSet<u8> {
        let mut out: std::collections::BTreeSet<u8> = plate
            .scene
            .objects
            .values()
            .map(|o| o.extruder_id.unwrap_or(1))
            .collect();
        for obj in plate.scene.objects.values() {
            out.extend(self.mesh_painted_materials(obj.mesh));
        }
        out
    }

    /// The MMU-painted filament states (`>= 1`) carried by `mesh`, or an empty
    /// set when the mesh is unknown or unpainted. Painted filaments are named
    /// only here — no object's `extruder_id` references them — so every
    /// "materials in use" derivation (rebind, orphan cleanup, the materials
    /// list) routes through this rather than reading objects alone.
    pub(crate) fn mesh_painted_materials(&self, mesh: MeshId) -> std::collections::BTreeSet<u8> {
        self.meshes
            .get(&mesh)
            .and_then(|m| m.paint_colors.as_ref())
            .map(|p| crate::core::threemf::referenced_states(p))
            .unwrap_or_default()
    }
}

impl Plate {
    /// Construct an empty plate (no printer assigned) at the
    /// given 1-based position with the default name.
    pub fn new(id: PlateId, position: u32) -> Self {
        Self {
            id,
            name: super::naming::plate_default_name(position),
            project_overrides: HashMap::new(),
            printer_instance_id: None,
            material_to_slot: std::collections::BTreeMap::new(),
            quality_profile: None,
            scene: PlateSceneState::default(),
        }
    }

    /// Construct a plate already bound to the named PrinterInstance.
    /// The bed is left unset; bind through [`Self::set_printer`] (or let
    /// `add_plate` populate it) to derive the bed from the printer.
    pub fn with_instance(id: PlateId, instance_id: String, position: u32) -> Self {
        Self {
            printer_instance_id: Some(instance_id),
            ..Self::new(id, position)
        }
    }

    /// The bound `PrinterInstance` id, or `None` when unbound.
    pub fn printer_instance_id(&self) -> Option<&str> {
        self.printer_instance_id.as_deref()
    }

    /// (Re)bind a plate's printer — sets the persisted instance id. The
    /// derived bed visualization follows this binding but is owned by
    /// `SessionRuntime`; callers reconcile the session (`Session::reconcile`)
    /// after a rebind so the bed re-derives. `None` unbinds.
    pub fn set_printer(&mut self, instance_id: Option<String>) {
        self.printer_instance_id = instance_id;
    }

    /// Length of the plate's materials list — the highest
    /// `extruder_id` (1-based model-material number) across the
    /// scene's objects, or 0 on an empty plate. This is the
    /// equivalent of BBS's project filament list length: the
    /// cascade composer fans out one libslic3r filament per
    /// material at index `material - 1`, and the `ams_mapping` /
    /// `ams_mapping2` arrays are sized the same so the firmware
    /// can look up `ams_mapping[material - 1]` for each tool
    /// change in the gcode.
    ///
    /// Objects without an explicit `extruder_id` are implicitly
    /// material 1 elsewhere in the pipeline. Pre-bound material
    /// numbers (in `material_to_slot`) without a corresponding
    /// object also extend the list — the user can pin a binding
    /// before adding geometry. An empty plate (no objects, no
    /// bindings) returns 0.
    pub fn material_count(&self) -> u8 {
        let mut max = 0u8;
        let mut has_any_object = false;
        for obj in self.scene.objects.values() {
            has_any_object = true;
            let mat = obj.extruder_id.unwrap_or(1);
            if mat > max {
                max = mat;
            }
        }
        // Also count materials explicitly bound (e.g. user pre-bound
        // a slot before adding the object). The cascade has to
        // include filaments for those too so the slice path doesn't
        // see a zero-length list when the only object hasn't loaded
        // yet.
        if let Some(&last_material) = self.material_to_slot.keys().next_back() {
            if last_material > max {
                max = last_material;
            }
        }
        if max == 0 && has_any_object {
            1
        } else {
            max
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const BAMBI: &str = "bambi";

    #[test]
    fn default_project_has_one_plate_no_cascade() {
        let p = Project::default();
        assert_eq!(p.plates.len(), 1);
        assert_eq!(p.plates[0].id, PlateId(1));
        assert_eq!(p.plates[0].name, "Plate 1");
        // Project::default is pure — an unbound bootstrap plate, no registry
        // access. The command layer binds the preferred/first printer through
        // `with_preferred_printer`.
        assert!(
            p.plates[0].printer_instance_id().is_none(),
            "default project is unbound; bootstrap binding is a command-layer concern",
        );
        assert_eq!(p.active_plate, 0);
        assert!(p.meshes.is_empty());
        assert_eq!(p.next_mesh_id, 0);
        assert_eq!(p.next_object_id, 0);
    }

    #[test]
    fn title_prefers_metadata_then_stem_then_untitled() {
        // Bare project: no Title metadata, no source path.
        let mut p = Project::default();
        assert_eq!(p.title(None), "Untitled");

        // A loaded source path (passed in — it lives in SessionRuntime now)
        // contributes its file stem.
        let src = PathBuf::from("/tmp/MyPrint.3mf");
        assert_eq!(p.title(Some(&src)), "MyPrint");

        // A non-empty Title metadata wins over the stem.
        p.file_metadata
            .insert("Title".into(), "Authored Name".into());
        assert_eq!(p.title(Some(&src)), "Authored Name");

        // A blank Title falls back to the stem (trimmed/empty is ignored).
        p.file_metadata.insert("Title".into(), "   ".into());
        assert_eq!(p.title(Some(&src)), "MyPrint");
    }

    #[test]
    fn plate_with_instance_constructor_attaches_id() {
        let p = Plate::with_instance(PlateId(2), BAMBI.into(), 2);
        assert_eq!(p.id, PlateId(2));
        assert_eq!(p.name, "Plate 2");
        assert_eq!(p.printer_instance_id(), Some(BAMBI));
    }

    #[test]
    fn with_preferred_printer_binds_named_else_first() {
        // Registry seeded with the bundled fixtures (bambi + snappy).
        let _guard = crate::core::printer::instance_registry::RegistryGuard::acquire();
        let instances = crate::core::printer::list_instances();

        // None → the first registered instance.
        let none = Project::with_preferred_printer(None, &instances);
        let first = none.plates[0].printer_instance_id().map(str::to_owned);
        assert!(first.is_some(), "None binds the first registered instance");

        // A valid preferred id binds that instance.
        let snappy = Project::with_preferred_printer(Some("snappy"), &instances);
        assert_eq!(snappy.plates[0].printer_instance_id(), Some("snappy"));

        // An unknown preferred id falls back to the first registered instance.
        let unknown = Project::with_preferred_printer(Some("does-not-exist"), &instances);
        assert_eq!(
            unknown.plates[0].printer_instance_id().map(str::to_owned),
            first,
            "unknown preferred falls back to the first instance",
        );
    }

    #[test]
    fn project_serde_round_trips() {
        let mut p = Project::default();
        p.plates[0].set_printer(Some(BAMBI.into()));
        p.plates[0]
            .project_overrides
            .insert("layer_height".into(), "0.12".into());
        p.user_overrides.insert("travel_speed".into(), "300".into());
        p.file_metadata
            .insert("Title".into(), "Test Project".into());

        let json = serde_json::to_string(&p).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.uuid, p.uuid);
        assert_eq!(parsed.plates.len(), 1);
        assert_eq!(parsed.plates[0].printer_instance_id(), Some(BAMBI));
        assert_eq!(
            parsed.plates[0]
                .project_overrides
                .get("layer_height")
                .map(|s| s.as_str()),
            Some("0.12"),
        );
        assert_eq!(
            parsed
                .user_overrides
                .get("travel_speed")
                .map(|s| s.as_str()),
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
        let plate = Plate::with_instance(PlateId(7), BAMBI.into(), 2);
        p.plates.push(plate);
        assert_eq!(p.plate(PlateId(7)).map(|x| x.id), Some(PlateId(7)));
        assert!(p.plate(PlateId(99)).is_none());
        assert_eq!(p.plate_index(PlateId(7)), Some(1));
        assert_eq!(p.plate_index(PlateId(99)), None);
    }
}
