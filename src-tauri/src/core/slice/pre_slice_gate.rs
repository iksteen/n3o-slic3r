//! Pre-slice validation gate (PR-S-7 — replaces the PR-5-6
//! gate that retired with the old MaterialBinding shape).
//!
//! Walks every plate the caller asked to slice and checks that the
//! per-plate `material_to_slot` map + the bound PrinterInstance are
//! coherent enough to produce a real slice:
//!
//! 1. Every model material referenced by an object on the plate has
//!    a `material_to_slot` entry. (Auto-bind plants one on object
//!    register; this catches plates loaded from disk before the
//!    auto-bind path existed, or plates where the user explicitly
//!    cleared an entry.)
//! 2. The mapped slot has a non-empty `filament_identity` on the
//!    bound PrinterInstance — Bambu's firmware refuses prints with
//!    an empty `filament_settings_id` in the CONFIG_BLOCK.
//! 3. Within a single extruder, the referenced slots do NOT mix
//!    `FeedKind::Direct` and `FeedKind::Ams` — Bambu can't pull
//!    from external + AMS in one job.
//!
//! Caller (the Tauri command layer / slice orchestrator) emits the
//! first failing plate's issue list; the frontend surfaces it on
//! the binding panel as inline errors.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::core::printer::{lookup_instance, FeedKind, SlotRef};
use crate::core::project::{PlateId, Project};

/// One plate's worth of validation issues, surfaced together so the
/// caller can render them as a unit on the binding panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlateValidationFailure {
    /// 1-based plate id matching the wire-side `plate_ids` entry
    /// the caller asked to slice.
    pub plate_id: u32,
    pub issues: Vec<SliceBlocker>,
}

/// A specific reason a plate isn't slice-ready. Frontend renders
/// each as an inline error on the matching panel surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SliceBlocker {
    /// Plate has no bound `PrinterInstance` — the slice command
    /// also catches this through `SliceInputError::UnboundPrinter`,
    /// but the gate surfaces it first so the binding panel can
    /// render a clean message.
    UnboundPrinter,
    /// `printer_instance_id` is set but the bundled registry doesn't
    /// know it. Symptomatic of a project loaded from disk against a
    /// PrinterInstance the user has since removed.
    UnknownPrinterInstance { instance_id: String },
    /// A model material referenced by an object on the plate has no
    /// entry in `material_to_slot`.
    UnmappedMaterial { model_material: u8 },
    /// `material_to_slot[material]` points to an extruder index out
    /// of range for the bound instance's topology. Shouldn't happen
    /// through the normal commands (`set_material_slot` range-checks)
    /// but loaded projects can carry stale references.
    SlotExtruderOutOfRange {
        model_material: u8,
        slot: SlotRef,
        extruders: usize,
    },
    /// Slot index out of range for the chosen extruder.
    SlotIndexOutOfRange {
        model_material: u8,
        slot: SlotRef,
        slots: usize,
    },
    /// The mapped slot has no filament bound — Bambu firmware
    /// rejects prints with empty filament_settings_id.
    SlotHasNoFilament {
        model_material: u8,
        slot: SlotRef,
    },
    /// Within one extruder, materials route to slots of mixed feed
    /// kinds (some `Direct`, some `Ams`). Bambu can't pull from
    /// both feed paths in one job.
    PerExtruderFeedMix {
        extruder: u8,
        direct_slot: u8,
        ams_slot: u8,
    },
}

/// Validate that every plate in `plate_ids` (that exists on
/// `project`) is ready to slice. Returns the **first** failing
/// plate; the slice command surfaces it as an error and the user
/// fixes that plate's bindings before a second attempt.
///
/// Plates absent from `project` are skipped — the orchestrator
/// today builds its libslic3r model from `input.model_path` on
/// disk, not from `Project`'s plate scene contents.
pub fn validate_pre_slice(
    project: &Project,
    plate_ids: &[u32],
) -> Result<(), PlateValidationFailure> {
    for &raw_id in plate_ids {
        let pid = PlateId(raw_id);
        let Some(plate) = project.plate(pid) else {
            continue;
        };
        let issues = validate_plate(plate);
        if !issues.is_empty() {
            return Err(PlateValidationFailure {
                plate_id: raw_id,
                issues,
            });
        }
    }
    Ok(())
}

/// Build the per-plate AMS binding list (`model_material → flat-slot
/// index`) the `.gcode.3mf` writer hands to the printer firmware.
///
/// Flat slot index is 1-based across the printer's `(extruder, slot)`
/// grid, extruder-major — slot 0 of extruder 0 = `1`, then slot 0
/// of extruder 1 = `2` (Snappy shape), or for A1+AMS Lite (1 × 5)
/// `Direct=1`, `AMS:1=2`, … `AMS:4=5`. Bambu firmware uses this
/// mapping to know which AMS slot to load for each filament index
/// in the gcode.
///
/// Empty when the plate's map is empty (single-material print on a
/// no-AMS printer doesn't need a mapping; firmware uses the only
/// loaded slot). Empty also when the plate is unbound — caller
/// shouldn't be reaching this path in that case, but defensively
/// no-op.
pub fn ams_bindings_for_plate(
    plate: &crate::core::project::model::Plate,
) -> Vec<crate::core::threemf::AmsBinding> {
    let Some(instance_id) = plate.printer_instance_id.as_deref() else {
        return Vec::new();
    };
    let Some(instance) = lookup_instance(instance_id) else {
        return Vec::new();
    };
    // Flatten the instance's slot grid once so we can convert
    // (extruder, slot) tuples into 1-based linear indices.
    let flat: BTreeMap<(u8, u8), u8> = instance
        .extruders
        .iter()
        .enumerate()
        .flat_map(|(e_idx, e)| {
            (0..e.slots.len()).map(move |s_idx| ((e_idx as u8, s_idx as u8), 0))
        })
        .enumerate()
        .map(|(linear, ((e, s), _))| ((e, s), (linear + 1) as u8))
        .collect();

    let mut out = Vec::new();
    for (&material, &slot_ref) in &plate.material_to_slot {
        if let Some(&ams_slot) = flat.get(&(slot_ref.extruder, slot_ref.slot)) {
            out.push(crate::core::threemf::AmsBinding {
                model_material_index: material,
                ams_slot,
            });
        }
    }
    out
}

fn validate_plate(plate: &crate::core::project::model::Plate) -> Vec<SliceBlocker> {
    let mut issues = Vec::new();

    let Some(instance_id) = plate.printer_instance_id.as_deref() else {
        issues.push(SliceBlocker::UnboundPrinter);
        return issues;
    };
    let Some(instance) = lookup_instance(instance_id) else {
        issues.push(SliceBlocker::UnknownPrinterInstance {
            instance_id: instance_id.to_owned(),
        });
        return issues;
    };

    // Pass 1: collect referenced materials from the scene.
    let mut referenced: BTreeSet<u8> = BTreeSet::new();
    for obj in plate.scene.objects.values() {
        let mat = obj.extruder_id.unwrap_or(1);
        if mat >= 1 {
            referenced.insert(mat);
        }
    }

    // Pass 2: each referenced material must have a slot, the slot
    // must be in range, and the slot must carry a filament.
    let mut per_extruder_feeds: BTreeMap<u8, (Option<u8>, Option<u8>)> = BTreeMap::new();
    for mat in &referenced {
        let Some(&slot_ref) = plate.material_to_slot.get(mat) else {
            issues.push(SliceBlocker::UnmappedMaterial { model_material: *mat });
            continue;
        };
        let extruders = instance.extruders.len();
        let Some(extruder) = instance.extruders.get(slot_ref.extruder as usize) else {
            issues.push(SliceBlocker::SlotExtruderOutOfRange {
                model_material: *mat,
                slot: slot_ref,
                extruders,
            });
            continue;
        };
        let slots = extruder.slots.len();
        let Some(slot) = extruder.slots.get(slot_ref.slot as usize) else {
            issues.push(SliceBlocker::SlotIndexOutOfRange {
                model_material: *mat,
                slot: slot_ref,
                slots,
            });
            continue;
        };
        if slot.filament_identity.as_deref().unwrap_or("").is_empty() {
            issues.push(SliceBlocker::SlotHasNoFilament {
                model_material: *mat,
                slot: slot_ref,
            });
        }
        // Per-extruder feed-mix tracking. First Direct + first Ams
        // slot indices per extruder; if both are populated when we
        // finish the loop, that extruder is in conflict.
        let entry = per_extruder_feeds.entry(slot_ref.extruder).or_insert((None, None));
        match slot.feed {
            FeedKind::Direct if entry.0.is_none() => entry.0 = Some(slot_ref.slot),
            FeedKind::Ams if entry.1.is_none() => entry.1 = Some(slot_ref.slot),
            _ => {}
        }
    }

    for (extruder_idx, (direct, ams)) in &per_extruder_feeds {
        if let (Some(d), Some(a)) = (direct, ams) {
            issues.push(SliceBlocker::PerExtruderFeedMix {
                extruder: *extruder_idx,
                direct_slot: *d,
                ams_slot: *a,
            });
        }
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::instance_registry::reset_to_bundled;
    use crate::core::printer::set_slot_filament;
    use crate::core::printer::SlotRef;
    use crate::core::scene::state::{MeshProvenance, NewMesh, NewSceneObject};
    use crate::core::scene::transform::Transform;
    use crate::core::printer::profile::BoundingBox;
    use std::sync::Mutex;

    // The bundled PrinterInstance registry is process-global mutable
    // state — serialize gate tests so concurrent runs don't see each
    // other's slot bindings.
    static GATE_LOCK: Mutex<()> = Mutex::new(());

    fn unit_cube() -> NewMesh {
        NewMesh {
            vertices: vec![0.0; 24],
            normals: vec![0.0; 24],
            indices: vec![0, 1, 2],
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            provenance: MeshProvenance::Primitive("cube".into()),
        }
    }

    fn add_cube(p: &mut Project, material: u8) {
        let mesh = p.register_mesh(unit_cube());
        p.register_object(NewSceneObject {
            mesh,
            transform: Transform::IDENTITY,
            name: format!("cube-m{material}"),
            visible: true,
            extruder_id: Some(material),
            parent: None,
        });
    }

    #[test]
    fn plate_with_all_referenced_materials_mapped_and_filaments_bound_passes() {
        let _g = GATE_LOCK.lock().unwrap();
        reset_to_bundled();
        set_slot_filament("bambi", 0, 0, Some("Generic PLA".into())).unwrap();
        let mut p = Project::default();
        add_cube(&mut p, 1);
        assert!(validate_pre_slice(&p, &[1]).is_ok());
        reset_to_bundled();
    }

    #[test]
    fn unbound_printer_blocks() {
        let _g = GATE_LOCK.lock().unwrap();
        reset_to_bundled();
        let mut p = Project::default();
        p.plates[0].printer_instance_id = None;
        add_cube(&mut p, 1);
        let err = validate_pre_slice(&p, &[1]).unwrap_err();
        assert!(err.issues.iter().any(|i| matches!(i, SliceBlocker::UnboundPrinter)));
        reset_to_bundled();
    }

    #[test]
    fn slot_without_filament_blocks() {
        let _g = GATE_LOCK.lock().unwrap();
        reset_to_bundled();
        // Bundled fixtures now ship every slot pre-bound to
        // `generic-pla`. Clear the slot auto-bind would target so
        // the gate sees an unbound slot in the material's path.
        set_slot_filament("bambi", 0, 0, None).unwrap();
        let mut p = Project::default();
        add_cube(&mut p, 1);
        let err = validate_pre_slice(&p, &[1]).unwrap_err();
        assert!(err
            .issues
            .iter()
            .any(|i| matches!(i, SliceBlocker::SlotHasNoFilament { .. })));
        reset_to_bundled();
    }

    #[test]
    fn unmapped_material_blocks_when_object_references_it() {
        let _g = GATE_LOCK.lock().unwrap();
        reset_to_bundled();
        set_slot_filament("bambi", 0, 0, Some("Generic PLA".into())).unwrap();
        let mut p = Project::default();
        add_cube(&mut p, 1);
        // Wipe the auto-bound mapping so we recover the
        // "user-deleted entry" path.
        p.plates[0].material_to_slot.clear();
        let err = validate_pre_slice(&p, &[1]).unwrap_err();
        assert!(err
            .issues
            .iter()
            .any(|i| matches!(i, SliceBlocker::UnmappedMaterial { model_material: 1 })));
        reset_to_bundled();
    }

    #[test]
    fn slot_extruder_out_of_range_blocks() {
        let _g = GATE_LOCK.lock().unwrap();
        reset_to_bundled();
        set_slot_filament("bambi", 0, 0, Some("Generic PLA".into())).unwrap();
        let mut p = Project::default();
        add_cube(&mut p, 1);
        // Plant a stale slot reference — pretend a project file
        // carries an extruder index that no longer exists.
        p.plates[0]
            .material_to_slot
            .insert(1, SlotRef { extruder: 5, slot: 0 });
        let err = validate_pre_slice(&p, &[1]).unwrap_err();
        assert!(err
            .issues
            .iter()
            .any(|i| matches!(i, SliceBlocker::SlotExtruderOutOfRange { .. })));
        reset_to_bundled();
    }

    #[test]
    fn absent_plate_id_is_skipped() {
        let _g = GATE_LOCK.lock().unwrap();
        reset_to_bundled();
        let p = Project::default();
        // No plate 99 → no error.
        assert!(validate_pre_slice(&p, &[99]).is_ok());
    }

    #[test]
    fn ams_bindings_pull_from_material_to_slot_map() {
        // Bambi has 1 extruder × 1 slot — material 1 lands on
        // (0, 0) which is flat slot 1.
        let _g = GATE_LOCK.lock().unwrap();
        reset_to_bundled();
        let mut p = Project::default();
        add_cube(&mut p, 1);
        let bindings = ams_bindings_for_plate(&p.plates[0]);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].model_material_index, 1);
        assert_eq!(bindings[0].ams_slot, 1);
        reset_to_bundled();
    }

    #[test]
    fn ams_bindings_flatten_extruder_grid_for_snappy() {
        // Snappy has 4 extruders × 1 slot — material N → extruder
        // (N-1 mod 4), slot 0 → flat slot N.
        let _g = GATE_LOCK.lock().unwrap();
        reset_to_bundled();
        let mut p = Project::default();
        // Snappy isn't the default — re-bind by hand.
        p.plates[0].printer_instance_id = Some("snappy".into());
        for mat in 1u8..=4 {
            add_cube(&mut p, mat);
        }
        let bindings = ams_bindings_for_plate(&p.plates[0]);
        assert_eq!(bindings.len(), 4);
        let by_mat: std::collections::BTreeMap<u8, u8> = bindings
            .iter()
            .map(|b| (b.model_material_index, b.ams_slot))
            .collect();
        assert_eq!(by_mat.get(&1), Some(&1)); // extruder 0 → flat 1
        assert_eq!(by_mat.get(&2), Some(&2)); // extruder 1 → flat 2
        assert_eq!(by_mat.get(&3), Some(&3));
        assert_eq!(by_mat.get(&4), Some(&4));
        reset_to_bundled();
    }
}
