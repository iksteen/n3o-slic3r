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
//!   project's cascade handle, and the requested plate id.
//! - The path to a freshly-written temp `.3mf` containing the
//!   plate's geometry. The caller (`slice_active_plate`
//!   Tauri command) is responsible for deleting it after the slice
//!   job's terminal event.
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
use std::path::PathBuf;

use crate::core::cascade::commands::{ContextJson, OverrideFileSpec};
use crate::core::filament;
use crate::core::filament::FilamentProfile;
use crate::core::printer::{self, lookup_instance, PrinterInstance};
use crate::core::project::{PlateId, Project};
use crate::core::scene::build_plate::{self, BuildPlate};
use crate::core::scene::state::NewMesh;
use crate::core::threemf::{project_from_objects, write_3mf, ProjectObject};

use super::job::SliceJobInput;

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
    UnsupportedBuildPlate {
        plate_id: PlateId,
        identity: String,
    },
    /// The plate has no objects. Slicing an empty plate is always
    /// the user's mistake — surface early rather than letting
    /// libslic3r emit "no geometry" two seconds in.
    EmptyScene { plate_id: PlateId },
    /// Writing the temp `.3mf` failed. The included path is the
    /// candidate the builder tried.
    TempWrite { path: PathBuf, message: String },
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
            Self::PrinterNotInRegistry { identity } => write!(
                f,
                "printer identity `{identity}` not in bundled registry",
            ),
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
            Self::TempWrite { path, message } => {
                write!(f, "couldn't write slice input at {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for SliceInputError {}

/// Build a `SliceJobInput` for `plate_id`. The temp `.3mf` written
/// to disk is returned alongside so the caller can delete it after
/// the slice job terminates.
///
/// `output_dir` becomes `SliceJobInput.output_dir` verbatim — the
/// caller decides where slice output (the `plate_<N>.gcode` files)
/// lands.
pub fn build_slice_input(
    project: &Project,
    plate_id: PlateId,
    output_dir: String,
) -> Result<(SliceJobInput, PathBuf), SliceInputError> {
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
        .printer_instance_id
        .clone()
        .ok_or(SliceInputError::UnboundPrinter { plate_id })?;
    let instance = lookup_instance(&printer_instance_id).ok_or(
        SliceInputError::UnboundPrinter { plate_id },
    )?;
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
        let mut filaments: Vec<FilamentProfile> =
            Vec::with_capacity(material_count);
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
    let user_overrides = encode_overrides_as_specs(
        "user-overrides.toml",
        &project.user_overrides,
    );
    let project_overrides = encode_overrides_as_specs(
        "project-overrides.toml",
        &plate.project_overrides,
    );

    // ── Temp 3MF write ────────────────────────────────────────
    //
    // Build the per-object extruder remap before writing the temp
    // .3mf. libslic3r reads each object's `extruder` metadata as the
    // 1-based *filament index* the object prints with; the per-print
    // `filament_map` then translates filament-idx → physical extruder
    // for toolchanger gcode + every `nozzle_temperature[i]`-style
    // template substitution (Snapmaker U1's machine_start_gcode is
    // `M104 T{initial_extruder} …` where `{initial_extruder}` is the
    // filament index — *not* the post-`filament_map` physical
    // extruder). To route "material N → T<m>" via the binding the
    // object's recorded extruder must become the flat slot index
    // corresponding to its bound slot; identity `filament_map` then
    // routes filament-idx-i to physical extruder i. The remap is
    // a project-side decision so it lives here, alongside the temp
    // .3mf write that's the last point before libslic3r consumes
    // the value.
    let temp_path = temp_3mf_path(plate_id);
    let project_3mf = build_plate_geometry(project, plate_id, &instance)
        .expect("plate existence checked above");
    write_3mf(&project_3mf, &temp_path).map_err(|e| SliceInputError::TempWrite {
        path: temp_path.clone(),
        message: format!("{e}"),
    })?;

    // ── Assemble the SliceJobInput ────────────────────────────
    let input = SliceJobInput {
        model_path: temp_path.to_string_lossy().into_owned(),
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
    };

    Ok((input, temp_path))
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

/// Filter `project.meshes` + the named plate's objects into a
/// geometry-only `Project3mf` ready for `write_3mf`. Returns `None`
/// if the plate id is unknown (caller checks this upstream).
///
/// Mesh filtering: only meshes referenced by this plate's objects
/// are included, so the temp file stays minimal even on
/// many-mesh projects.
///
/// `instance` is borrowed to remap per-object `extruder_id` on
/// toolchanger printers (where the cascade is per-slot and the
/// gcode emits the filament index directly). On AMS-style
/// printers the remap is identity — the cascade is per-material
/// and material number ⇔ filament index already.
fn build_plate_geometry(
    project: &Project,
    plate_id: PlateId,
    instance: &PrinterInstance,
) -> Option<crate::core::threemf::Project3mf> {
    let plate = project.plates.iter().find(|p| p.id == plate_id)?;

    // Walk objects to determine which meshes are actually used; this
    // keeps the temp file small even when the project carries
    // meshes other plates use exclusively.
    let mut used_mesh_ids: Vec<_> = plate
        .scene
        .objects
        .values()
        .map(|o| o.mesh)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    used_mesh_ids.sort();

    let mesh_id_to_idx: HashMap<_, _> = used_mesh_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();

    let geometry_meshes: Vec<NewMesh> = used_mesh_ids
        .iter()
        .map(|id| {
            let m = &project.meshes[id];
            NewMesh {
                vertices: m.vertices.clone(),
                normals: m.normals.clone(),
                indices: m.indices.clone(),
                bounding_box: m.bounding_box,
                provenance: m.provenance.clone(),
            }
        })
        .collect();

    let geometry_objects: Vec<ProjectObject> = plate
        .scene
        .objects
        .values()
        .map(|obj| {
            // Remap material → libslic3r filament index. For
            // toolchangers this routes via the bound slot's
            // flat-slot index (legacy behavior — the per-slot
            // cascade puts each slot's filament settings at that
            // position). For AMS-style printers this is identity
            // (material N → filament N) because the per-material
            // cascade already places M N's settings at filament
            // index N - 1.
            let material = obj.extruder_id.unwrap_or(1);
            let remapped = material_to_filament_idx(material, instance, &plate.material_to_slot);
            ProjectObject {
                mesh_idx: mesh_id_to_idx[&obj.mesh],
                transform: obj.transform,
                name: obj.name.clone(),
                extruder_id: Some(remapped),
                // Plate id collapses to 1 in the temp file — libslic3r
                // only sees one plate per slice job; the multi-plate
                // shape is project-level, not slice-input-level.
                plate_id: 1,
                // Preserve group identity into the temp .3mf so the
                // writer collapses multi-volume groups (BBS-style
                // single ModelObject with N ModelVolumes) instead of
                // emitting each volume as a freestanding object —
                // otherwise libslic3r flags non-bed-touching volumes
                // as "floating regions" needing supports.
                group_id: obj.group_id,
            }
        })
        .collect();

    Some(project_from_objects(
        geometry_meshes,
        geometry_objects,
        BTreeMap::new(), // file metadata is project-level, slicing doesn't need it
    ))
}

/// Encode a flat key-value override map as a TOML body the cascade's
/// `parse_override_str` will accept. Keys sorted for deterministic
/// output (helps reproducibility + lets tests pin exact strings).
///
/// Returns an empty `Vec` for empty input — the cascade's override
/// parser is happy with a zero-spec list.
fn encode_overrides_as_specs(
    label: &str,
    map: &HashMap<String, String>,
) -> Vec<OverrideFileSpec> {
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

/// Build a unique temp-file path scoped to the requesting plate +
/// process + monotonic nanos. command deletes this file on
/// the slice job's terminal event.
fn temp_3mf_path(plate_id: PlateId) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "n3o-slice-plate{}-{}-{}.3mf",
        plate_id.0,
        std::process::id(),
        nanos,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::instance_registry::RegistryGuard;
    use crate::core::scene::state::{MeshProvenance, NewMesh, NewSceneObject};
    use crate::core::scene::transform::Transform;
    use crate::core::printer::profile::BoundingBox;

    fn triangle_mesh() -> NewMesh {
        NewMesh {
            vertices: vec![0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 10.0, 0.0],
            normals: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
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
        p.plates[0].printer_instance_id = Some("bambi".into());
        let mesh_id = p.register_mesh(triangle_mesh());
        p.register_object(NewSceneObject::at_origin(mesh_id, "cube"));
        p
    }

    #[test]
    fn happy_path_builds_input_and_writes_temp_3mf() {
        let _registry = RegistryGuard::acquire();
        let project = one_plate_project_with_cube();
        let (input, temp_path) =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into())
                .expect("build");

        assert_eq!(input.plate_ids, vec![1]);
        assert_eq!(input.context.printer.model, "Bambu Lab A1 mini");
        // The bambi instance ships with Supertack Plate; reads off the
        // instance, not off a per-binding override.
        assert_eq!(input.context.plate.identity, "Supertack Plate");
        assert_eq!(input.context.plate.libslic3r_curr_bed_type, "Supertack Plate");
        assert!(temp_path.exists(), "temp file written");
        assert_eq!(input.model_path, temp_path.to_string_lossy());

        // One filament per *material* on the plate. This single-cube
        // happy path uses material 1 only → length 1, sourced from
        // bambi's first AMS slot (generic-pla in the bundled
        // fixture). Slot count is independent.
        assert_eq!(input.context.filaments.len(), 1);
        assert_eq!(input.context.filaments[0].identity, "generic-pla");
        assert_eq!(input.material_layout.len(), 1);
        assert!(input.material_layout[0].is_some(), "M1 auto-binds to an AMS slot");

        std::fs::remove_file(&temp_path).ok();
    }

    #[test]
    fn multi_plate_targets_the_requested_plate_not_active() {
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();

        // Plate 1: A1 mini with one cube.
        project.plates[0].printer_instance_id = Some("bambi".into());
        let mesh_a = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject::at_origin(mesh_a, "cube-a"));

        // Plate 2: Snapmaker U1 with one cube. Activate so
        // register_object lands on it.
        let (id2, _) = project.add_plate(None);
        project.plates[1].printer_instance_id = Some("snappy".into());
        project.set_active_plate(id2).expect("activate plate 2");
        let mesh_b = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject::at_origin(mesh_b, "cube-b"));

        // Build for plate 2 explicitly.
        let (input, temp_path) =
            build_slice_input(&project, id2, "/tmp/n3o-out".into()).expect("build plate 2");
        assert_eq!(input.plate_ids, vec![2]);
        assert_eq!(input.context.printer.model, "Snapmaker U1");
        assert_eq!(input.context.plate.identity, "Textured PEI Plate");
        assert_eq!(input.context.plate.libslic3r_curr_bed_type, "Textured PEI Plate");
        // Snappy is a toolchanger (>1 extruder) → legacy per-slot
        // cascade: 4 extruders × 1 slot = 4 filaments. AMS-style
        // printers (single extruder) would use one filament per
        // material instead.
        assert_eq!(input.context.filaments.len(), 4);
        for f in &input.context.filaments {
            assert_eq!(f.identity, "generic-pla");
        }

        std::fs::remove_file(&temp_path).ok();
    }

    #[test]
    fn per_object_extruder_passes_material_through_verbatim() {
        // Per-material cascade (BBS convention): libslic3r filament
        // index ⇔ model material number. The object's authored
        // `extruder_id` (= material number) passes through verbatim
        // to the temp `.3mf` so libslic3r emits `T<material - 1>`,
        // and the per-material filament_settings_id / filament_map
        // entries the composer fans out at material's position carry
        // the bound slot's filament identity + extruder routing.
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].printer_instance_id = Some("bambi".into());
        let mesh_id = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::IDENTITY,
            name: "cube-m3".into(),
            visible: true,
            extruder_id: Some(3),
            parent: None,
            group_id: None,
        });

        let (input, temp_path) =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");

        let reloaded =
            crate::core::threemf::load_3mf(&temp_path).expect("reload temp 3MF");
        assert_eq!(reloaded.objects.len(), 1);
        assert_eq!(reloaded.objects[0].extruder_id, Some(3));
        // material_layout has one entry per material; material 3 → 3
        // entries, and the third entry is the auto-bound AMS slot.
        assert_eq!(input.material_layout.len(), 3);
        assert!(input.material_layout[2].is_some());

        std::fs::remove_file(&temp_path).ok();
    }

    #[test]
    fn per_object_extruder_remaps_for_snappy_toolchanger() {
        // Snappy = toolchanger → legacy "one filament per slot"
        // cascade with the per-object remap reapplied. Bind M1 →
        // T1's solo slot (extruder=1, slot=0): flat slot index 1,
        // libslic3r filament index 2 (1-based). The temp `.3mf`
        // carries the remapped value so libslic3r's template
        // substitution picks the right filament.
        use crate::core::printer::SlotRef;
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].printer_instance_id = Some("snappy".into());
        let mesh_id = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::IDENTITY,
            name: "cube-m1".into(),
            visible: true,
            extruder_id: Some(1),
            parent: None,
            group_id: None,
        });
        // Override the auto-bind with an explicit "M1 → T1" binding.
        project.plates[0]
            .material_to_slot
            .insert(1, SlotRef { extruder: 1, slot: 0 });

        let (input, temp_path) =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");

        let reloaded =
            crate::core::threemf::load_3mf(&temp_path).expect("reload temp 3MF");
        assert_eq!(reloaded.objects.len(), 1);
        assert_eq!(reloaded.objects[0].extruder_id, Some(2));
        // Toolchangers fall back to the legacy slot-fanned cascade
        // — `material_layout` stays empty so `compose_cascade` uses
        // `slot_layout` (4 filaments for snappy).
        assert!(input.material_layout.is_empty());

        std::fs::remove_file(&temp_path).ok();
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

        let (input, temp_path) =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");

        assert_eq!(input.context.user_overrides.len(), 1);
        assert!(input.context.user_overrides[0]
            .content
            .contains("travel_speed = \"300\""));
        assert_eq!(input.context.project_overrides.len(), 1);
        assert!(input.context.project_overrides[0]
            .content
            .contains("layer_height = \"0.12\""));

        std::fs::remove_file(&temp_path).ok();
    }

    #[test]
    fn empty_override_maps_produce_empty_spec_lists() {
        let _registry = RegistryGuard::acquire();
        let project = one_plate_project_with_cube();
        let (input, temp_path) =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");
        assert!(input.context.user_overrides.is_empty());
        assert!(input.context.project_overrides.is_empty());
        std::fs::remove_file(&temp_path).ok();
    }

    #[test]
    fn grouped_objects_emit_as_one_modelobject_in_temp_3mf() {
        // Multi-volume groups (the cube-halves shape) must round-trip
        // through build_slice_input → temp .3mf as ONE outer object
        // with N parts. Otherwise libslic3r flags non-bed-touching
        // volumes as "floating regions" needing supports — exactly
        // the bug we hit on the cube-halves external-spool fixture.
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].printer_instance_id = Some("bambi".into());
        let mesh_id = project.register_mesh(triangle_mesh());
        // Two objects sharing group_id=42 with distinct extruder
        // hints — same shape the cube-halves loader produces.
        project.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::IDENTITY,
            name: "lower".into(),
            visible: true,
            extruder_id: Some(1),
            parent: None,
            group_id: Some(42),
        });
        project.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::translation(glam::Vec3::new(0.0, 0.0, 10.0)),
            name: "upper".into(),
            visible: true,
            extruder_id: Some(2),
            parent: None,
            group_id: Some(42),
        });

        let (_input, temp_path) =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");

        // Inspect the written temp .3mf — one build item, one
        // <components> wrapper with two <component>s, one outer
        // model_settings object with two parts.
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&temp_path).unwrap()).unwrap();
        let mut model_xml = String::new();
        std::io::Read::read_to_string(
            &mut zip.by_name("3D/3dmodel.model").unwrap(),
            &mut model_xml,
        )
        .unwrap();
        let mut settings_xml = String::new();
        std::io::Read::read_to_string(
            &mut zip.by_name("Metadata/model_settings.config").unwrap(),
            &mut settings_xml,
        )
        .unwrap();
        assert_eq!(model_xml.matches("<item ").count(), 1);
        assert_eq!(model_xml.matches("<components>").count(), 1);
        assert_eq!(model_xml.matches("<component ").count(), 2);
        assert_eq!(settings_xml.matches("<object id=").count(), 1);
        assert_eq!(settings_xml.matches("<part ").count(), 2);

        std::fs::remove_file(&temp_path).ok();
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
        project.plates[0].printer_instance_id = None;
        project.plates[0].scene.bed = None;
        let mesh_id = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject::at_origin(mesh_id, "cube"));
        let err = build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into())
            .expect_err("no printer bound");
        assert!(matches!(
            err,
            SliceInputError::UnboundPrinter { plate_id: PlateId(1) }
        ));
    }

    #[test]
    fn empty_scene_errors() {
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].printer_instance_id = Some("bambi".into());
        // No register_object call → no objects on the plate.
        let err = build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into())
            .expect_err("empty scene");
        assert!(matches!(
            err,
            SliceInputError::EmptyScene { plate_id: PlateId(1) }
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
        project.plates[0].printer_instance_id = Some("bambi".into());
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
        assert!(matches!(
            err,
            SliceInputError::UnsupportedBuildPlate { .. }
        ));
    }

    #[test]
    fn snappy_emits_one_filament_per_extruder_slot() {
        // Snappy is a toolchanger → legacy slot-fanned cascade
        // (4 extruders × 1 slot). Each slot is seeded with the
        // bundled `generic-pla` fragment.
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].printer_instance_id = Some("snappy".into());
        let mesh_id = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject::at_origin(mesh_id, "cube"));

        let (input, temp_path) =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");
        assert_eq!(input.context.filaments.len(), 4);
        for f in &input.context.filaments {
            assert_eq!(f.identity, "generic-pla");
        }
        std::fs::remove_file(&temp_path).ok();
    }

    #[test]
    fn temp_3mf_omits_unused_meshes_from_other_plates() {
        let _registry = RegistryGuard::acquire();
        let mut project = Project::default();
        project.plates[0].printer_instance_id = Some("bambi".into());

        // Mesh on plate 1.
        let mesh_a = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject::at_origin(mesh_a, "a"));

        // Plate 2 with its own mesh.
        let (id2, _) = project.add_plate(None);
        project.plates[1].printer_instance_id = Some("bambi".into());
        project.set_active_plate(id2).unwrap();
        let mesh_b = project.register_mesh(triangle_mesh());
        project.register_object(NewSceneObject::at_origin(mesh_b, "b"));

        // Build for plate 1; the temp file should carry only mesh_a.
        let (_, temp_path) =
            build_slice_input(&project, PlateId(1), "/tmp/n3o-out".into()).expect("build");
        let reloaded =
            crate::core::threemf::load_3mf(&temp_path).expect("reload");
        assert_eq!(reloaded.meshes.len(), 1, "plate 2's mesh excluded");
        assert_eq!(reloaded.objects.len(), 1);
        assert_eq!(reloaded.objects[0].name, "a");

        std::fs::remove_file(&temp_path).ok();
    }
}
