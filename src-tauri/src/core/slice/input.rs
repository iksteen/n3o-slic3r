//! Scene-to-slice input builder.
//!
//! Pure adapter that turns the live [`Project`]'s per-plate state
//! into a [`SliceJobInput`] libslic3r can consume. Replaces the
//! path-based "user picks a mesh file" flow that ignored everything
//! the user composed in the scene.
//!
//! Output:
//! - A populated [`SliceJobInput`] carrying the resolved cascade
//!   context (printer + build plate + filaments + overrides), the
//!   project's cascade handle, the requested plate id, and the plate's
//!   geometry as [`SliceObject`]s — `Arc`-shared mesh buffers fed
//!   straight to libslic3r's `Model` in-memory (no temp `.3mf`).
//!
//! What's NOT here:
//! - Per-object override propagation through ContextJson — the
//!   cascade's `object_overrides` field is scoped to a single
//!   "active object" at resolve time, while a slice run may touch
//!   N objects with N distinct override sets. Wiring that through
//!   the orchestrator's per-object resolve loop is a separate
//!   ticket. For now `object_overrides` is left empty; project +
//!   user tier overrides still apply globally.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::core::cascade::commands::{ContextJson, OverrideFileSpec};
use crate::core::filament;
use crate::core::filament::FilamentProfile;
use crate::core::printer::{self, lookup_instance, PrinterInstance};
use crate::core::project::{PlateId, Project};
use crate::core::scene::build_plate::{self, BuildPlate};
use crate::core::scene::state::{GroupId, ModifierKind};

use super::job::SliceJobInput;

/// One scene object's geometry, ready to hand to libslic3r's `Model`
/// in-memory via `Model::add_object` (no temp `.3mf`). The mesh buffers
/// are `Arc`-shared from the project's [`Mesh`](crate::core::scene::state::Mesh)
/// — building this list is a cheap pointer-bump, not a geometry copy.
#[derive(Debug, Clone)]
pub struct SliceObject {
    pub name: String,
    /// Object-local, flat XYZ vertices, shared from the mesh.
    pub vertices: Arc<Vec<f32>>,
    /// Flat triangle vertex indices (3 per triangle), shared from the mesh.
    pub indices: Arc<Vec<u32>>,
    /// Per-triangle BBS `paint_color` hex (MMU painting), shared from the
    /// mesh; `None` when the mesh is unpainted.
    pub paint: Option<Arc<Vec<String>>>,
    /// Object→world transform, column-major (from `Transform.matrix`).
    pub transform: [f64; 16],
    /// 1-based libslic3r filament index, post material→filament remap.
    pub extruder: i32,
    /// Per-object config overrides (scope-gated to object/region keys).
    pub overrides: Vec<(String, String)>,
    /// Multi-volume group identity (see
    /// [`SceneObject::group`](crate::core::scene::state::SceneObject::group)).
    /// Objects sharing a `GroupId` are collapsed into one ModelObject in-memory
    /// (`add_group` + one `add_volume` per member) by the slice worker — no
    /// temp `.3mf` needed.
    pub group: Option<GroupId>,
    /// Cut-connector volumes (pegs/holes) on this object, in the same local
    /// frame as the part. Applied as extra libslic3r volumes (peg = MODEL_PART,
    /// hole = NEGATIVE_VOLUME) so an object with any becomes one multi-volume
    /// ModelObject. Empty for ordinary objects.
    pub modifiers: Vec<SliceModifier>,
}

/// A cut connector carried into the slice: peg or hole geometry + its role.
#[derive(Debug, Clone)]
pub struct SliceModifier {
    pub vertices: Arc<Vec<f32>>,
    pub indices: Arc<Vec<u32>>,
    /// `true` = NEGATIVE_VOLUME (hole, subtracted); `false` = MODEL_PART (peg).
    pub negative: bool,
}

/// Failure modes for [`build_slice_input`]. Caller (the Tauri
/// command layer) maps each to a user-visible error string.
#[derive(Debug)]
pub enum SliceInputError {
    /// `plate_id` doesn't exist in `project.plates`.
    UnknownPlate(PlateId),
    /// The plate has no printer binding — picker must run
    /// first.
    UnboundPrinter { plate_id: PlateId },
    /// Printer identity isn't in the bundled registry. Symptomatic
    /// of a loaded project authored against a printer this build
    /// doesn't ship; UI should prompt to rebind to a bundled one.
    PrinterNotInRegistry { identity: String },
    /// The PrinterInstance's currently-loaded bed isn't in the bound
    /// printer's `supported_build_plates`. Shouldn't happen for
    /// instances mutated through the normal commands (those validate
    /// at set-time) but a hand-edited on-disk instance file could
    /// trip it.
    UnsupportedBuildPlate { plate_id: PlateId, identity: String },
    /// The plate has no objects. Slicing an empty plate is always
    /// the user's mistake — surface early rather than letting
    /// libslic3r emit "no geometry" two seconds in.
    EmptyScene { plate_id: PlateId },
}

impl std::fmt::Display for SliceInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownPlate(id) => write!(f, "plate {} is unknown", id.0),
            Self::UnboundPrinter { plate_id } => write!(
                f,
                "plate {} has no printer bound; pick one first",
                plate_id.0,
            ),
            Self::PrinterNotInRegistry { identity } => {
                write!(f, "printer identity `{identity}` not in bundled registry",)
            }
            Self::UnsupportedBuildPlate { plate_id, identity } => write!(
                f,
                "plate {}: build plate `{identity}` not supported by the bound printer",
                plate_id.0,
            ),
            Self::EmptyScene { plate_id } => write!(
                f,
                "plate {} has no objects; add geometry before slicing",
                plate_id.0,
            ),
        }
    }
}

impl std::error::Error for SliceInputError {}

/// Build a `SliceJobInput` for `plate_id`, carrying the plate's
/// geometry as [`SliceObject`]s (`Arc`-shared mesh buffers) the worker
/// hands to libslic3r in-memory — no temp `.3mf` on the default path.
///
/// `output_dir` becomes `SliceJobInput.output_dir` verbatim — the
/// caller decides where slice output (the `plate_<N>.gcode` files)
/// lands.
pub fn build_slice_input(
    project: &Project,
    plate_id: PlateId,
    output_dir: String,
) -> Result<SliceJobInput, SliceInputError> {
    // ── Plate lookup ──────────────────────────────────────────
    let plate = project
        .plates
        .iter()
        .find(|p| p.id == plate_id)
        .ok_or(SliceInputError::UnknownPlate(plate_id))?;

    // ── Printer instance routing ──────────────────────────────
    // Cascade composition happens in the orchestrator from this
    // instance's per-bucket vendor fragments. The composer is the
    // only slice path; an unbound plate (no printer_instance_id)
    // can't slice. The printer profile is derived from the
    // instance's `vendor_profile_ref` — the per-plate binding has
    // no separate identity of its own.
    let printer_instance_id = plate
        .printer_instance_id()
        .map(str::to_owned)
        .ok_or(SliceInputError::UnboundPrinter { plate_id })?;
    let instance = lookup_instance(&printer_instance_id)
        .ok_or(SliceInputError::UnboundPrinter { plate_id })?;
    let printer_profile = printer::lookup(&instance.vendor_profile_ref).ok_or_else(|| {
        SliceInputError::PrinterNotInRegistry {
            identity: instance.vendor_profile_ref.clone(),
        }
    })?;

    // ── Build plate ──────────────────────────────────────────
    // The PrinterInstance is the single source of truth for which
    // bed is currently loaded. `printer_instance_set_bed` validates
    // against `supported_build_plates` at set time, so we only have
    // a defense-in-depth check for hand-edited on-disk instances.
    let bed_identity = instance.bed.identity.clone();
    if !printer_profile
        .supported_build_plates
        .iter()
        .any(|p| p == &bed_identity)
    {
        return Err(SliceInputError::UnsupportedBuildPlate {
            plate_id,
            identity: bed_identity,
        });
    }
    let build_plate = build_plate::lookup(&bed_identity).unwrap_or_else(|| {
        // Synthesized fallback for plates we accept in
        // `supported_build_plates` but don't have a TOML asset for
        // yet (e.g. snapmaker U1's "Magnetic"). The cascade still
        // needs a `libslic3r_curr_bed_type` to write into the slice
        // config; a best-effort `"<identity> Plate"` keeps libslic3r
        // happy without authoring real plate profiles up-front.
        BuildPlate {
            identity: bed_identity.clone(),
            libslic3r_curr_bed_type: format!("{} Plate", bed_identity),
        }
    });

    // ── Empty-scene check (before materials assembly so the
    //    fallback for zero-material plates isn't needed) ──────
    if plate.scene.objects.is_empty() {
        return Err(SliceInputError::EmptyScene { plate_id });
    }

    // ── Material layout — one filament position per material ─────
    //
    // BBS convention for AMS-style printers (single physical
    // extruder + AMS slots): the libslic3r cascade has one filament
    // per material in the user's materials list, ordered by
    // material index. Position `i` belongs to model material
    // `i + 1`. The gcode emits `T<material - 1>` and the driver's
    // `ams_mapping[material - 1]` routes the bound spool to it.
    //
    // Toolchangers (one physical extruder per toolhead) are
    // different: libslic3r's tool-change template emits the
    // filament index directly as the T number, and the firmware
    // takes that as the physical toolhead selector — no AMS
    // mapping in between. To keep "material N → T<bound>" working
    // we keep the legacy "one filament per (extruder, slot)"
    // cascade for toolchangers and remap each object's
    // `extruder_id` to its bound extruder's flat-slot index at
    // `.3mf` write time.
    //
    // `instance.extruders.len() > 1` is the marker — AMS-style
    // printers always carry exactly one extruder regardless of
    // AMS unit count.
    let is_toolchanger = instance.extruders.len() > 1;
    let material_count = plate.material_count() as usize;
    let (material_layout, filaments) = if is_toolchanger {
        // Legacy slot-fanned cascade — empty `material_layout`
        // makes `compose_cascade` fall back to `slot_layout`.
        // Per-slot filament profiles match the cascade's filament
        // dimension.
        let mut filaments: Vec<FilamentProfile> = Vec::new();
        for extruder in &instance.extruders {
            for slot in &extruder.slots {
                let identity = slot
                    .filament_identity
                    .as_deref()
                    .unwrap_or(instance.default_filament_fragment_slug.as_str());
                let profile = filament::lookup(identity).unwrap_or_else(|| FilamentProfile {
                    identity: identity.to_owned(),
                    base_type: "PLA".into(),
                    vendor: None,
                    color: None,
                });
                filaments.push(profile);
            }
        }
        (Vec::new(), filaments)
    } else {
        // Per-material cascade: one filament per material, slot
        // bindings resolved via `material_to_slot`. Unbound
        // materials fall back to the instance's
        // `default_filament_fragment_slug` so the cascade is
        // always resolvable, even before the user picks a slot.
        let mut layout: Vec<Option<crate::core::printer::SlotRef>> =
            Vec::with_capacity(material_count);
        let mut filaments: Vec<FilamentProfile> = Vec::with_capacity(material_count);
        for material in 1..=material_count as u8 {
            let slot_ref = plate.material_to_slot.get(&material).copied();
            layout.push(slot_ref);
            let identity = slot_ref
                .and_then(|sr| instance.extruders.get(sr.extruder as usize))
                .and_then(|ext| {
                    let slot_idx = match slot_ref {
                        Some(sr) => sr.slot as usize,
                        None => return None,
                    };
                    ext.slots.get(slot_idx)
                })
                .and_then(|slot| slot.filament_identity.as_deref())
                .unwrap_or(instance.default_filament_fragment_slug.as_str());
            let profile = filament::lookup(identity).unwrap_or_else(|| FilamentProfile {
                identity: identity.to_owned(),
                base_type: "PLA".into(),
                vendor: None,
                color: None,
            });
            filaments.push(profile);
        }
        (layout, filaments)
    };

    // ── Overrides ─────────────────────────────────────────────
    let user_overrides = encode_overrides_as_specs("user-overrides.toml", &project.user_overrides);
    let project_overrides =
        encode_overrides_as_specs("project-overrides.toml", &plate.project_overrides);

    // ── Geometry ──────────────────────────────────────────────
    //
    // Build the per-object geometry + extruder remap. libslic3r reads
    // each object's `extruder` as the 1-based *filament index* the
    // object prints with; the per-print `filament_map` then translates
    // filament-idx → physical extruder for toolchanger gcode + every
    // `nozzle_temperature[i]`-style template substitution (Snapmaker
    // U1's machine_start_gcode is `M104 T{initial_extruder} …` where
    // `{initial_extruder}` is the filament index — *not* the
    // post-`filament_map` physical extruder). To route "material N →
    // T<m>" via the binding the object's recorded extruder must become
    // the flat slot index corresponding to its bound slot; identity
    // `filament_map` then routes filament-idx-i to physical extruder i.
    // [`build_plate_objects`] applies that remap into each object's
    // `extruder`.
    let objects = build_plate_objects(project, plate_id, &instance);

    // ── MMU paint remap (toolchangers only) ───────────────────
    // build_plate_objects rewrites each object's `extruder` to its flat-
    // slot index on toolchangers. The face paint encodes the *original* model
    // material indices, so it needs the same remap or painted faces route to
    // the wrong toolhead. AMS printers remap identity, and an unpainted plate
    // needs nothing → `None` for both (and the orchestrator skips the call).
    let plate_has_paint = project.plate_has_painted_object(plate);
    let paint_filament_remap = if is_toolchanger && plate_has_paint {
        // perm[state]: state 0 (the object's own extruder) maps to itself;
        // each painted material 1..=N maps to its flat-slot filament index,
        // matching the per-object `extruder_id` remap.
        let mut perm: Vec<i32> = (0..=material_count as i32).collect();
        for m in 1..=material_count as u8 {
            perm[m as usize] =
                material_to_filament_idx(m, &instance, &plate.material_to_slot) as i32;
        }
        Some(perm)
    } else {
        None
    };

    // ── Assemble the SliceJobInput ────────────────────────────
    let input = SliceJobInput {
        objects,
        output_dir,
        context: ContextJson {
            printer: printer_profile,
            plate: build_plate,
            filaments,
            active_slot: 0,
            user_overrides,
            project_overrides,
            // See module docs: per-object overrides need a wider
            // refactor to flow through slicing. Empty here keeps
            // global resolve correct; per-object overrides are
            // ignored at slice time until that lands.
            object_overrides: HashMap::new(),
        },
        plate_ids: vec![plate_id.0],
        printer_instance_id,
        material_layout,
        quality_profile: plate.quality_profile.clone(),
        paint_filament_remap,
    };

    Ok(input)
}

/// Map a model-material number to the 1-based libslic3r filament
/// index for a *toolchanger* instance, honouring
/// `plate.material_to_slot`. AMS-style printers get identity
/// (material N → filament N) because the per-material cascade
/// makes `filament_index == material - 1`.
///
/// On a toolchanger the legacy "one filament per (extruder, slot)"
/// cascade is in effect, so the material's bound slot translates
/// to a flat slot index that libslic3r reads as the filament index.
/// `filament_map` stays identity downstream, so the gcode emits
/// `T<filament_idx - 1>` for the right physical extruder. Without
/// a binding, fall back to identity (material N → filament N) —
/// the slicer treats unbound material 1 as filament 1.
fn material_to_filament_idx(
    material: u8,
    instance: &PrinterInstance,
    material_to_slot: &std::collections::BTreeMap<u8, crate::core::printer::SlotRef>,
) -> u8 {
    if instance.extruders.len() <= 1 {
        // AMS-style — per-material cascade owns the mapping.
        return material;
    }
    if let Some(slot_ref) = material_to_slot.get(&material) {
        // Sum slot counts of preceding extruders + slot index = flat
        // 0-based, then +1 for libslic3r's 1-based filament index.
        let preceding: usize = instance
            .extruders
            .iter()
            .take(slot_ref.extruder as usize)
            .map(|e| e.slots.len())
            .sum();
        (preceding + slot_ref.slot as usize + 1) as u8
    } else {
        material
    }
}

/// The per-object overrides libslic3r can actually honor for `id`: the
/// stored override map filtered (via the shared [`schema::gate_object_overrides`]
/// gate) to object/region-scope keys, so only settings the engine applies
/// per object reach libslic3r. Returned as a `BTreeMap` so the override
/// order is deterministic.
fn object_overrides_for_slice(
    object_overrides: &HashMap<crate::core::scene::state::ObjectId, HashMap<String, String>>,
    id: crate::core::scene::state::ObjectId,
) -> BTreeMap<String, String> {
    match object_overrides.get(&id) {
        Some(raw) => crate::core::schema::gate_object_overrides(raw, id.0),
        None => BTreeMap::new(),
    }
}

/// Turn the named plate's objects into [`SliceObject`]s the worker
/// hands to libslic3r in-memory. Empty when the plate id is unknown
/// (the caller checks this upstream).
///
/// The mesh buffers are `Arc`-cloned (pointer-bump, no geometry copy).
/// Objects are emitted in the plate's authored order (ObjectList
/// preserves it) — that order is what libslic3r sees, and where two
/// objects of different materials overlap, the *last* one wins, so a
/// stable order keeps the overlap region's filament stable between
/// slices.
///
/// `instance` is borrowed to remap each object's `extruder` on
/// toolchanger printers (where the cascade is per-slot and the gcode
/// emits the filament index directly). On AMS-style printers the remap
/// is identity — the cascade is per-material and material number ⇔
/// filament index already.
pub fn build_plate_objects(
    project: &Project,
    plate_id: PlateId,
    instance: &PrinterInstance,
) -> Vec<SliceObject> {
    let Some(plate) = project.plates.iter().find(|p| p.id == plate_id) else {
        return Vec::new();
    };

    plate
        .scene
        .objects
        .values()
        .map(|obj| {
            let mesh = &project.meshes[&obj.mesh];
            // Remap material → libslic3r filament index. For
            // toolchangers this routes via the bound slot's flat-slot
            // index (the per-slot cascade puts each slot's filament
            // settings at that position). For AMS-style printers this is
            // identity (material N → filament N) because the per-material
            // cascade already places M N's settings at filament index
            // N - 1.
            let material = obj.extruder_id.unwrap_or(1);
            let extruder =
                material_to_filament_idx(material, instance, &plate.material_to_slot) as i32;
            SliceObject {
                name: obj.name.clone(),
                vertices: Arc::clone(&mesh.vertices),
                indices: Arc::clone(&mesh.indices),
                paint: mesh.paint_colors.clone(),
                transform: obj.transform.matrix.map(f64::from),
                extruder,
                overrides: object_overrides_for_slice(&plate.scene.object_overrides, obj.id)
                    .into_iter()
                    .collect(),
                group: obj.group,
                modifiers: plate
                    .scene
                    .object_modifiers
                    .get(&obj.id)
                    .into_iter()
                    .flatten()
                    .filter_map(|m| {
                        let mesh = project.meshes.get(&m.mesh)?;
                        Some(SliceModifier {
                            vertices: Arc::clone(&mesh.vertices),
                            indices: Arc::clone(&mesh.indices),
                            negative: matches!(m.kind, ModifierKind::Hole),
                        })
                    })
                    .collect(),
            }
        })
        .collect()
}

/// Encode a flat key-value override map as a TOML body the cascade's
/// `parse_override_str` will accept. Keys sorted for deterministic
/// output (helps reproducibility + lets tests pin exact strings).
///
/// Returns an empty `Vec` for empty input — the cascade's override
/// parser is happy with a zero-spec list.
fn encode_overrides_as_specs(label: &str, map: &HashMap<String, String>) -> Vec<OverrideFileSpec> {
    if map.is_empty() {
        return Vec::new();
    }
    // Sort keys for deterministic output. The `toml` crate
    // serializes `BTreeMap` in key order, so that's the easy path.
    let sorted: BTreeMap<&String, &String> = map.iter().collect();
    let content = toml::to_string(&sorted).unwrap_or_else(|e| {
        // toml::to_string on String→String maps is infallible in
        // practice; if it ever fails the override would silently
        // drop, so panic loudly instead.
        panic!("encode_overrides_as_specs({label}): {e}")
    });
    vec![OverrideFileSpec {
        label: label.into(),
        content,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::instance_registry::RegistryGuard;
    use crate::core::printer::profile::BoundingBox;
    use crate::core::scene::state::{MeshProvenance, NewMesh, NewSceneObject, ObjectId};
    use crate::core::scene::transform::Transform;

    #[test]
    fn object_overrides_for_slice_keeps_object_scope_drops_print_and_unknown() {
        // Schema scope comes from the FFI option table.
        let _ = slic3r_ffi::init(None, 3);
        let id = ObjectId(7);
        let mut inner = HashMap::new();
        inner.insert("layer_height".to_string(), "0.3".to_string()); // PrintObjectConfig → object: kept
        inner.insert("skirt_loops".to_string(), "2".to_string()); // PrintConfig → print scope: dropped
        inner.insert("n3o_not_a_real_option".to_string(), "x".to_string()); // not a libslic3r key: dropped
        let map = HashMap::from([(id, inner)]);

        assert_eq!(
            object_overrides_for_slice(&map, id),
            BTreeMap::from([("layer_height".to_string(), "0.3".to_string())]),
            "only the object/region-scoped key survives the gate",
        );
        // An object with no stored overrides yields an empty map.
        assert!(object_overrides_for_slice(&map, ObjectId(999)).is_empty());
    }

    fn triangle_mesh() -> NewMesh {
        NewMesh {
            vertices: vec![0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 10.0, 0.0],
            indices: vec![0, 1, 2],
            paint_colors: None,
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [10.0, 10.0, 0.0],
            },
            provenance: MeshProvenance::Primitive("tri".into()),
        }
    }

    fn one_plate_project_with_cube() -> Project {
        let mut p = Project::default();
        // Project::default() auto-binds the bootstrap plate to the
        // bundled default printer (Bambi) — pin it explicitly so the
        // tests don't drift if the bundled-default identity changes.
        p.plates[0].set_printer(Some("bambi".into()), None);
        let mesh_id = p.register_mesh(triangle_mesh());
        p.register_object(NewSceneObject::at_origin(mesh_id, "cube"));
        p
    }

    #[test]
    fn happy_path_builds_input_with_objects() {
        let _registry = RegistryGuard::acquire();
        let project = one_plate_project_with_cube();
        let input =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");

        assert_eq!(input.plate_ids, vec![1]);
        assert_eq!(input.context.printer.model, "Bambu Lab A1 mini");
        // The bambi instance ships with Supertack Plate; reads off the
        // instance, not off a per-binding override.
        assert_eq!(input.context.plate.identity, "Supertack Plate");
        assert_eq!(
            input.context.plate.libslic3r_curr_bed_type,
            "Supertack Plate"
        );
        // Geometry travels in-memory as one SliceObject per scene object,
        // with shared buffers — geometry travels in-memory, no temp file.
        assert_eq!(input.objects.len(), 1);
        assert_eq!(input.objects[0].name, "cube");
        assert!(!input.objects[0].vertices.is_empty());
        assert_eq!(input.objects[0].extruder, 1);

        // One filament per *material* on the plate. This single-cube
        // happy path uses material 1 only → length 1, sourced from
        // bambi's first AMS slot (generic-pla in the bundled
        // fixture). Slot count is independent.
        assert_eq!(input.context.filaments.len(), 1);
        assert_eq!(input.context.filaments[0].identity, "generic-pla");
        assert_eq!(input.material_layout.len(), 1);
        assert!(
            input.material_layout[0].is_some(),
            "M1 auto-binds to an AMS slot"
        );
    }

    #[test]
    fn multi_plate_targets_the_requested_plate_not_active() {
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();

        // Plate 1: A1 mini with one cube.
        project.plates[0].set_printer(Some("bambi".into()), None);
        let mesh_a = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject::at_origin(mesh_a, "cube-a"));

        // Plate 2: Snapmaker U1 with one cube. Activate so
        // register_object lands on it.
        let (id2, _) = project.add_plate(None);
        project.plates[1].set_printer(Some("snappy".into()), None);
        project.set_active_plate(id2).expect("activate plate 2");
        let mesh_b = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject::at_origin(mesh_b, "cube-b"));

        // Build for plate 2 explicitly.
        let input =
            build_slice_input(&project, id2, "/tmp/n3o-out".into()).expect("build plate 2");
        assert_eq!(input.plate_ids, vec![2]);
        assert_eq!(input.context.printer.model, "Snapmaker U1");
        assert_eq!(input.context.plate.identity, "Textured PEI Plate");
        assert_eq!(
            input.context.plate.libslic3r_curr_bed_type,
            "Textured PEI Plate"
        );
        // Plate 2's single object only — plate 1's geometry isn't here.
        assert_eq!(input.objects.len(), 1);
        assert_eq!(input.objects[0].name, "cube-b");
        // Snappy is a toolchanger (>1 extruder) → legacy per-slot
        // cascade: 4 extruders × 1 slot = 4 filaments. AMS-style
        // printers (single extruder) would use one filament per
        // material instead.
        assert_eq!(input.context.filaments.len(), 4);
        for f in &input.context.filaments {
            assert_eq!(f.identity, "generic-pla");
        }
    }

    #[test]
    fn per_object_extruder_passes_material_through_verbatim() {
        // Per-material cascade (BBS convention): libslic3r filament
        // index ⇔ model material number. The object's authored
        // `extruder_id` (= material number) passes through verbatim
        // to the SliceObject so libslic3r emits `T<material - 1>`,
        // and the per-material filament_settings_id / filament_map
        // entries the composer fans out at material's position carry
        // the bound slot's filament identity + extruder routing.
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].set_printer(Some("bambi".into()), None);
        let mesh_id = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::IDENTITY,
            name: "cube-m3".into(),
            visible: true,
            extruder_id: Some(3),
            group: None,
        });

        let input =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");

        assert_eq!(input.objects.len(), 1);
        assert_eq!(input.objects[0].extruder, 3);
        // material_layout has one entry per material; material 3 → 3
        // entries, and the third entry is the auto-bound AMS slot.
        assert_eq!(input.material_layout.len(), 3);
        assert!(input.material_layout[2].is_some());
    }

    #[test]
    fn per_object_extruder_remaps_for_snappy_toolchanger() {
        // Snappy = toolchanger → legacy "one filament per slot"
        // cascade with the per-object remap reapplied. Bind M1 →
        // T1's solo slot (extruder=1, slot=0): flat slot index 1,
        // libslic3r filament index 2 (1-based). The SliceObject
        // carries the remapped value so libslic3r's template
        // substitution picks the right filament.
        use crate::core::printer::SlotRef;
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].set_printer(Some("snappy".into()), None);
        let mesh_id = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::IDENTITY,
            name: "cube-m1".into(),
            visible: true,
            extruder_id: Some(1),
            group: None,
        });
        // Override the auto-bind with an explicit "M1 → T1" binding.
        project.plates[0].material_to_slot.insert(
            1,
            SlotRef {
                extruder: 1,
                slot: 0,
            },
        );

        let input =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");

        assert_eq!(input.objects.len(), 1);
        assert_eq!(input.objects[0].extruder, 2);
        // Toolchangers fall back to the legacy slot-fanned cascade
        // — `material_layout` stays empty so `compose_cascade` uses
        // `slot_layout` (4 filaments for snappy).
        assert!(input.material_layout.is_empty());
    }

    #[test]
    fn project_and_user_overrides_populate_context_specs() {
        let _registry = RegistryGuard::acquire();
        let mut project = one_plate_project_with_cube();
        project
            .user_overrides
            .insert("travel_speed".into(), "300".into());
        project.plates[0]
            .project_overrides
            .insert("layer_height".into(), "0.12".into());

        let input =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");

        assert_eq!(input.context.user_overrides.len(), 1);
        assert!(input.context.user_overrides[0]
            .content
            .contains("travel_speed = \"300\""));
        assert_eq!(input.context.project_overrides.len(), 1);
        assert!(input.context.project_overrides[0]
            .content
            .contains("layer_height = \"0.12\""));
    }

    #[test]
    fn empty_override_maps_produce_empty_spec_lists() {
        let _registry = RegistryGuard::acquire();
        let project = one_plate_project_with_cube();
        let input =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");
        assert!(input.context.user_overrides.is_empty());
        assert!(input.context.project_overrides.is_empty());
    }

    #[test]
    fn grouped_plate_uses_in_memory_group_path() {
        // Multi-volume groups (the cube-halves shape) slice in-memory like
        // everything else: the worker collapses the group into one ModelObject
        // via `add_group` + `add_volume`. The objects carry their group identity
        // so the worker can do that collapse.
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].set_printer(Some("bambi".into()), None);
        let mesh_id = project.register_mesh(triangle_mesh());
        // Two objects sharing one group with distinct extruder hints —
        // same shape the cube-halves loader produces.
        let g = crate::core::scene::state::GroupId::fresh();
        project.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::IDENTITY,
            name: "lower".into(),
            visible: true,
            extruder_id: Some(1),
            group: Some(g),
        });
        project.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::translation(glam::Vec3::new(0.0, 0.0, 10.0)),
            name: "upper".into(),
            visible: true,
            extruder_id: Some(2),
            group: Some(g),
        });

        let input =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");

        assert_eq!(input.objects.len(), 2);
        assert!(
            input.objects.iter().all(|o| o.group == Some(g)),
            "both objects keep their group identity for the worker to collapse \
             into one ModelObject (add_group + add_volume)",
        );
    }

    #[test]
    fn unknown_plate_id_errors() {
        let _registry = RegistryGuard::acquire();
        let project = one_plate_project_with_cube();
        let err = build_slice_input(&project, PlateId(99), "/tmp/n3o-out".into())
            .expect_err("plate 99 not present");
        assert!(matches!(err, SliceInputError::UnknownPlate(PlateId(99))));
    }

    #[test]
    fn unbound_printer_errors() {
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        // Project::default now auto-binds; clear it so this test
        // pins the genuinely-unbound error path.
        project.plates[0].set_printer(None, None);
        project.plates[0].scene.bed = None;
        let mesh_id = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject::at_origin(mesh_id, "cube"));
        let err = build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into())
            .expect_err("no printer bound");
        assert!(matches!(
            err,
            SliceInputError::UnboundPrinter {
                plate_id: PlateId(1)
            }
        ));
    }

    #[test]
    fn empty_scene_errors() {
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].set_printer(Some("bambi".into()), None);
        // No register_object call → no objects on the plate.
        let err = build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into())
            .expect_err("empty scene");
        assert!(matches!(
            err,
            SliceInputError::EmptyScene {
                plate_id: PlateId(1)
            }
        ));
    }

    #[test]
    fn unsupported_build_plate_errors() {
        // The supported-plate validation in `printer_instance_set_bed`
        // makes this path unreachable through normal mutations; the
        // defense-in-depth check in `build_slice_input` exists for the
        // hand-edited on-disk instance case. Simulate that by going
        // around the validator via `mutate_instance` directly.
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].set_printer(Some("bambi".into()), None);
        let mesh_id = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject::at_origin(mesh_id, "cube"));

        printer::mutate_instance("bambi", |inst| {
            // A1 mini doesn't support U1's Magnetic plate.
            inst.bed.identity = "Magnetic".into();
            Ok(())
        })
        .unwrap();
        let err = build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into())
            .expect_err("a1 mini doesn't support magnetic plate");
        // No manual restore: `RegistryGuard::Drop` resets to bundled
        // before the next test sees the registry, regardless of any
        // failure path through this body.
        assert!(matches!(err, SliceInputError::UnsupportedBuildPlate { .. }));
    }

    #[test]
    fn snappy_emits_one_filament_per_extruder_slot() {
        // Snappy is a toolchanger → legacy slot-fanned cascade
        // (4 extruders × 1 slot). Each slot is seeded with the
        // bundled `generic-pla` fragment.
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].set_printer(Some("snappy".into()), None);
        let mesh_id = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject::at_origin(mesh_id, "cube"));

        let input =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");
        assert_eq!(input.context.filaments.len(), 4);
        for f in &input.context.filaments {
            assert_eq!(f.identity, "generic-pla");
        }
    }

    #[test]
    fn plate_objects_exclude_other_plates_objects() {
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].set_printer(Some("bambi".into()), None);

        // Mesh on plate 1.
        let mesh_a = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject::at_origin(mesh_a, "a"));

        // Plate 2 with its own mesh.
        let (id2, _) = project.add_plate(None);
        project.plates[1].set_printer(Some("bambi".into()), None);
        project.set_active_plate(id2).unwrap();
        let mesh_b = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject::at_origin(mesh_b, "b"));

        // Build for plate 1; only plate 1's object is carried.
        let input =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");
        assert_eq!(input.objects.len(), 1);
        assert_eq!(input.objects[0].name, "a");
    }

    #[test]
    fn objects_emit_in_stable_id_order() {
        // The SliceObject order is what libslic3r sees — and for
        // overlapping objects of different materials the last one wins. The
        // slice path emits in the plate's authored (ObjectList) order; this
        // pins that the order survives (here: creation order).
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].set_printer(Some("bambi".into()), None);
        let mesh = project.register_mesh(triangle_mesh());
        for i in 0..8 {
            project.register_object(NewSceneObject::at_origin(mesh, format!("obj-{i}")));
        }

        let input =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");
        let names: Vec<&str> = input.objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(
            names,
            ["obj-0", "obj-1", "obj-2", "obj-3", "obj-4", "obj-5", "obj-6", "obj-7"],
            "objects must slice in ascending-id (creation) order",
        );
    }
}
