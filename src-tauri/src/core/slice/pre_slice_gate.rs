//! Pre-slice validation gate.
//!
//! Walks every plate the caller asked to slice and checks that the
//! per-plate `material_to_slot` map + the bound PrinterInstance are
//! coherent enough to produce a real slice:
//!
//! 1. Every model material referenced by an object on the plate has
//!    a `material_to_slot` entry. (Auto-bind plants one on object
//!    register; this catches the edge case where the user explicitly
//!    cleared an entry or a project file from disk lacks one.)
//! 2. The mapped slot has a non-empty `filament_identity` on the
//!    bound PrinterInstance — Bambu's firmware refuses prints with
//!    an empty `filament_settings_id` in the CONFIG_BLOCK.
//!
//! The slice orchestrator surfaces the first failing plate's issue
//! list; the frontend renders inline errors on the binding panel.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::core::printer::{lookup_instance, SlotRef};
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
    SlotHasNoFilament { model_material: u8, slot: SlotRef },
}

/// Validate that every plate in `plate_ids` (that exists on
/// `project`) is ready to slice. Returns the **first** failing
/// plate; the slice command surfaces it as an error and the user
/// fixes that plate's bindings before a second attempt.
///
/// Plates absent from `project` are skipped — the orchestrator builds
/// its libslic3r model from the slice input's in-memory geometry
/// buffers, snapshotted from `Project`'s plate scene at build time.
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

fn validate_plate(plate: &crate::core::project::model::Plate) -> Vec<SliceBlocker> {
    let mut issues = Vec::new();

    let Some(instance_id) = plate.printer_instance_id() else {
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
    for mat in &referenced {
        let Some(&slot_ref) = plate.material_to_slot.get(mat) else {
            issues.push(SliceBlocker::UnmappedMaterial {
                model_material: *mat,
            });
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
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::instance_registry::RegistryGuard;
    use crate::core::printer::profile::BoundingBox;
    use crate::core::printer::set_slot_filament;
    use crate::core::printer::SlotRef;
    use crate::core::scene::state::{MeshProvenance, NewMesh, NewSceneObject};
    use crate::core::scene::transform::Transform;

    fn unit_cube() -> NewMesh {
        NewMesh {
            vertices: vec![0.0; 24],
            indices: vec![0, 1, 2],
            paint_colors: None,
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
            group: None,
        });
    }

    #[test]
    fn plate_with_all_referenced_materials_mapped_and_filaments_bound_passes() {
        let _registry = RegistryGuard::acquire();
        set_slot_filament("bambi", 0, 0, Some("Generic PLA".into())).unwrap();
        let mut p = Project::default();
        add_cube(&mut p, 1);
        assert!(validate_pre_slice(&p, &[1]).is_ok());
    }

    #[test]
    fn unbound_printer_blocks() {
        let _registry = RegistryGuard::acquire();
        let mut p = Project::default();
        p.plates[0].set_printer(None, None);
        add_cube(&mut p, 1);
        let err = validate_pre_slice(&p, &[1]).unwrap_err();
        assert!(err
            .issues
            .iter()
            .any(|i| matches!(i, SliceBlocker::UnboundPrinter)));
    }

    #[test]
    fn slot_without_filament_blocks() {
        let _registry = RegistryGuard::acquire();
        // Bundled fixtures now ship every slot pre-bound to
        // `generic-pla`. AMS-first ordering means auto-bind lands
        // material 1 on AMS:1 = (extruder 0, slot 0) — clear that
        // slot's filament so the gate sees an unbound slot in the
        // material's path.
        set_slot_filament("bambi", 0, 0, None).unwrap();
        let mut p = Project::default();
        add_cube(&mut p, 1);
        let err = validate_pre_slice(&p, &[1]).unwrap_err();
        assert!(err
            .issues
            .iter()
            .any(|i| matches!(i, SliceBlocker::SlotHasNoFilament { .. })));
    }

    #[test]
    fn unmapped_material_blocks_when_object_references_it() {
        let _registry = RegistryGuard::acquire();
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
    }

    #[test]
    fn slot_extruder_out_of_range_blocks() {
        let _registry = RegistryGuard::acquire();
        set_slot_filament("bambi", 0, 0, Some("Generic PLA".into())).unwrap();
        let mut p = Project::default();
        add_cube(&mut p, 1);
        // Plant a stale slot reference — pretend a project file
        // carries an extruder index that no longer exists.
        p.plates[0].material_to_slot.insert(
            1,
            SlotRef {
                extruder: 5,
                slot: 0,
            },
        );
        let err = validate_pre_slice(&p, &[1]).unwrap_err();
        assert!(err
            .issues
            .iter()
            .any(|i| matches!(i, SliceBlocker::SlotExtruderOutOfRange { .. })));
    }

    #[test]
    fn absent_plate_id_is_skipped() {
        let _registry = RegistryGuard::acquire();
        let p = Project::default();
        // No plate 99 → no error.
        assert!(validate_pre_slice(&p, &[99]).is_ok());
    }
}
