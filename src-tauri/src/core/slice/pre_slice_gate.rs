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

/// Build the per-plate AMS binding list (`model_material → 1-based
/// AMS slot`) the `.gcode.3mf` writer hands to the printer firmware.
///
/// The `ams_slot` is the 1-based position of the slot among the
/// *AMS-feed* slots of its extruder — Direct/external slots are
/// skipped. For Bambi (AMS-first ordering: `[AMS:1, AMS:2, AMS:3,
/// AMS:4, Ext]`) the values are `AMS:1=1 .. AMS:4=4`. These
/// become the `M620 S<N>A` operands the firmware fires to load
/// the spool into the hotend.
///
/// Empty when the plate's map is empty (single-material print on a
/// no-AMS printer doesn't need a mapping; firmware uses the only
/// loaded slot). Empty also when the plate is unbound — caller
/// shouldn't be reaching this path in that case, but defensively
/// no-op.
pub fn ams_bindings_for_plate(
    plate: &crate::core::project::model::Plate,
) -> Vec<crate::core::threemf::AmsBinding> {
    use crate::core::printer::FeedKind;
    let Some(instance_id) = plate.printer_instance_id.as_deref() else {
        return Vec::new();
    };
    let Some(instance) = lookup_instance(instance_id) else {
        return Vec::new();
    };

    // BBL firmware expects 1-based AMS-only slot indices (1..N for
    // an N-slot AMS unit) in the `ams_bindings.ams_slot` field —
    // these become the `M620 S<N>A` operands the print pre-loads.
    //
    // **Two rules this loop enforces:**
    //   1. Direct-fed slots (the A1 mini's external spool) aren't
    //      AMS-addressable. Material loading from a Direct slot is
    //      signaled via the separate `ams_mapping` field's `-1`
    //      sentinel and must NOT appear in ams_bindings at all.
    //   2. The slot number is 1-based among AMS-feed slots only —
    //      not the flat slot grid. Numbering the flat grid (Ext
    //      counted as slot 0 or 1) inflates every AMS index by one
    //      and pushes AMS:4 to ams_slot=5, which doesn't exist on
    //      the 4-slot AMS lite; firmware refuses to load that with
    //      "filament loading error" (observed on a real 4-color
    //      print).
    let mut out = Vec::new();
    for (&material, &slot_ref) in &plate.material_to_slot {
        let Some(extruder) = instance.extruders.get(slot_ref.extruder as usize) else {
            continue;
        };
        let Some(slot) = extruder.slots.get(slot_ref.slot as usize) else {
            continue;
        };
        if slot.feed != FeedKind::Ams {
            continue;
        }
        let ams_slot = extruder.slots[..=slot_ref.slot as usize]
            .iter()
            .filter(|s| s.feed == FeedKind::Ams)
            .count() as u8;
        out.push(crate::core::threemf::AmsBinding {
            model_material_index: material,
            ams_slot,
        });
    }
    out
}

/// One entry of the Bambu MQTT `ams_mapping2` array — the
/// `{ams_id, slot_id}` form the firmware uses to identify which
/// physical AMS unit + slot within it a filament loads from.
/// Unused entries are `{ams_id: 255, slot_id: 255}` (the sentinel
/// BBS publishes for empty positions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AmsMappingV2 {
    pub ams_id: u8,
    pub slot_id: u8,
}

impl AmsMappingV2 {
    pub const UNUSED: Self = Self {
        ams_id: 255,
        slot_id: 255,
    };
}

/// Compute the Bambu MQTT `project_file` AMS routing fields for a
/// plate: `(use_ams, ams_mapping, ams_mapping2)`.
///
/// Both arrays are sized to the plate's **materials list length**
/// (`plate.material_count()`) — the same equivalence BBS uses for
/// its `ams_mapping` arrays (length = project filament list length).
/// Filament index `i` corresponds to model material `i + 1`. The
/// cascade composer pairs this: one libslic3r filament per
/// material, ordered by material index. The slicer emits `T<n>`
/// where `n` is the 0-based material index, and the firmware uses
/// `ams_mapping[n]` + `ams_mapping2[n]` to route to the right
/// physical spool.
///
/// `ams_mapping[i]` encoding:
///   - `0..3` — the 0-based AMS slot index within the unit when
///     material `i + 1` is bound to an Ams-feed slot
///   - `-1` — material `i + 1` is bound to a Direct-feed slot
///     (external spool) OR isn't bound to any slot
///
/// `ams_mapping2[i]` encoding:
///   - `{ams_id, slot_id}` with `ams_id = 0..N-1` when material
///     `i + 1` is bound to an Ams-feed slot
///   - `{255, 0}` — material `i + 1` is bound to a Direct-feed
///     slot (the Ext sentinel BBS publishes)
///   - `{255, 255}` — material `i + 1` isn't bound to any slot
///     (no tool change for this material — the firmware ignores
///     the position)
///
/// Both shapes are required: the firmware on A1 mini consumes them
/// in tandem, and BBS always publishes both. `use_ams` is true when
/// at least one referenced material lands on an `Ams`-feed slot.
///
/// Indexed by material position, NOT by PrinterInstance slot
/// position. Captured BBS traffic confirms the per-material rule:
///   - 4-material AMS print (M1..M4 → AMS:1..4): `[0,1,2,3]`
///     (BBS sent length 5 because their project had a 5th
///     unbound material in the list; we size to the materials
///     actually present)
///   - 3-material AMS + M4-on-Ext (M1..M3 → AMS:1..3, M4 → Ext):
///     `[0,1,2,-1]` with `mapping2[3] = {255, 0}`
///   - 2-material M1-on-Ext + M2-on-AMS:1: `[-1, 0]` with
///     `mapping2[0] = {255, 0}`, exact match to BBS
///
/// MVP: we assume a single AMS unit per extruder (the A1 mini + AMS
/// Lite case), so `ams_id` is always `0` for AMS-fed slots. Multi-
/// AMS printers (X1C with 4 AMS units) need a richer printer-side
/// model to know which AMS each `Ams`-feed slot belongs to.
pub fn ams_mapping_for_plate(
    plate: &crate::core::project::model::Plate,
) -> (bool, Vec<i8>, Vec<AmsMappingV2>) {
    use crate::core::printer::FeedKind;
    let Some(instance_id) = plate.printer_instance_id.as_deref() else {
        return (false, Vec::new(), Vec::new());
    };
    let Some(instance) = lookup_instance(instance_id) else {
        return (false, Vec::new(), Vec::new());
    };

    // Length = materials list length. Material N occupies filament
    // index N - 1. Materials present in the list but not bound to
    // any slot stay at `-1` / `{255, 255}`. An empty plate (no
    // materials) produces empty arrays.
    let material_count = plate.material_count() as usize;
    let mut mapping = vec![-1i8; material_count];
    let mut mapping2 = vec![AmsMappingV2::UNUSED; material_count];
    let mut any_ams = false;
    for (&material, slot_ref) in &plate.material_to_slot {
        if material < 1 {
            continue;
        }
        let filament_index = (material as usize) - 1;
        if filament_index >= mapping.len() {
            continue;
        }
        let Some(ext) = instance.extruders.get(slot_ref.extruder as usize) else {
            continue;
        };
        let Some(slot) = ext.slots.get(slot_ref.slot as usize) else {
            continue;
        };
        match slot.feed {
            FeedKind::Ams => {
                let ams_slot = ext.slots[..slot_ref.slot as usize]
                    .iter()
                    .filter(|s| s.feed == FeedKind::Ams)
                    .count() as u8;
                mapping[filament_index] = ams_slot as i8;
                mapping2[filament_index] = AmsMappingV2 {
                    ams_id: 0,
                    slot_id: ams_slot,
                };
                any_ams = true;
            }
            FeedKind::Direct => {
                // Ext sentinel: mapping stays `-1`, mapping2 carries
                // `{255, 0}` so the firmware distinguishes "bound to
                // external spool" from "padding/unused".
                mapping[filament_index] = -1;
                mapping2[filament_index] = AmsMappingV2 {
                    ams_id: 255,
                    slot_id: 0,
                };
            }
        }
    }
    (any_ams, mapping, mapping2)
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
            group_id: None,
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
        p.plates[0].printer_instance_id = None;
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

    #[test]
    fn ams_bindings_pull_from_material_to_slot_map() {
        // Bambi has 1 extruder × 5 slots (AMS:1..AMS:4 + Ext). Auto-
        // bind skips the external spool when AMS slots exist, so
        // material 1 lands on (0, 0) = AMS:1 → ams_slot=1 (first
        // AMS-feed slot under this extruder, 1-based).
        let _registry = RegistryGuard::acquire();
        let mut p = Project::default();
        add_cube(&mut p, 1);
        let bindings = ams_bindings_for_plate(&p.plates[0]);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].model_material_index, 1);
        assert_eq!(bindings[0].ams_slot, 1);
    }

    #[test]
    fn ams_bindings_for_full_amslite_uses_1_through_4_not_2_through_5() {
        // Regression for the off-by-one that caused a real-print
        // filament-loading error on the 4-cube 4-mat smoke: M1..M4
        // got published as ams_slot 2,3,4,5 (Ext was being counted
        // as ams_slot=1). M4→5 doesn't exist on the AMS lite's
        // 4 slots; the firmware refused to load it.
        let _registry = RegistryGuard::acquire();
        let mut p = Project::default();
        for mat in 1u8..=4 {
            add_cube(&mut p, mat);
        }
        let bindings = ams_bindings_for_plate(&p.plates[0]);
        let by_mat: std::collections::BTreeMap<u8, u8> = bindings
            .iter()
            .map(|b| (b.model_material_index, b.ams_slot))
            .collect();
        assert_eq!(by_mat.get(&1), Some(&1));
        assert_eq!(by_mat.get(&2), Some(&2));
        assert_eq!(by_mat.get(&3), Some(&3));
        assert_eq!(by_mat.get(&4), Some(&4));
        assert!(
            !by_mat.values().any(|&v| v == 5),
            "no AMS slot may publish as 5 — the AMS lite only has 4 slots",
        );
    }

    #[test]
    fn ams_bindings_skips_materials_routed_to_direct_external_spool() {
        // Material bound explicitly to the Bambi external spool
        // (Direct-feed slot at index 4 in the AMS-first ordering)
        // should NOT publish an ams_bindings entry — the firmware
        // reads "use external spool" via the separate `ams_mapping`
        // field's `-1` sentinel. Publishing it as an AMS slot would
        // route the print to a real AMS slot the user didn't
        // choose.
        use crate::core::printer::SlotRef;
        let _registry = RegistryGuard::acquire();
        let mut p = Project::default();
        // Override the auto-bind: pin M1 to Ext.
        add_cube(&mut p, 1);
        p.plates[0].material_to_slot.insert(
            1,
            SlotRef {
                extruder: 0,
                slot: 4,
            },
        );
        let bindings = ams_bindings_for_plate(&p.plates[0]);
        assert!(bindings.is_empty(), "got {bindings:?}");
    }

    #[test]
    fn ams_mapping_for_4mat_bambi_indexed_by_material_position() {
        // 4-material AMS print (M1..M4 → AMS:1..4 via auto-bind on
        // the AMS-first bambi slot layout). The mapping arrays are
        // sized to the materials list length (= 4) and indexed by
        // material position: filament_index = material - 1. This
        // matches BBS's captured `project_file` for the same print:
        //   ams_mapping  = [0, 1, 2, 3]
        //   ams_mapping2 = [{0,0}, {0,1}, {0,2}, {0,3}]
        // (BBS sent length 5 because their project had a 5th unused
        // material in the filament list; we size to the materials
        // actually present.)
        let _registry = RegistryGuard::acquire();
        let mut p = Project::default();
        for mat in 1u8..=4 {
            add_cube(&mut p, mat);
        }
        let (use_ams, mapping, mapping2) = ams_mapping_for_plate(&p.plates[0]);
        assert!(use_ams);
        assert_eq!(mapping, vec![0, 1, 2, 3]);
        assert_eq!(
            mapping2[0],
            AmsMappingV2 {
                ams_id: 0,
                slot_id: 0
            }
        );
        assert_eq!(
            mapping2[1],
            AmsMappingV2 {
                ams_id: 0,
                slot_id: 1
            }
        );
        assert_eq!(
            mapping2[2],
            AmsMappingV2 {
                ams_id: 0,
                slot_id: 2
            }
        );
        assert_eq!(
            mapping2[3],
            AmsMappingV2 {
                ams_id: 0,
                slot_id: 3
            }
        );
    }

    #[test]
    fn ams_mapping_with_ext_in_middle_emits_ext_sentinel_at_material_position() {
        // 4-material print, M1..M3 on AMS, M4 on Ext. Captured BBS
        // shape (modulo padding for an unused 5th BBS material):
        //   ams_mapping  = [0, 1, 2, -1]
        //   ams_mapping2 = [{0,0}, {0,1}, {0,2}, {255,0}]
        // The `{255, 0}` (Ext sentinel) at index 3 distinguishes
        // "bound to external spool" from "padding/unused" (which
        // would be `{255, 255}`).
        use crate::core::printer::SlotRef;
        let _registry = RegistryGuard::acquire();
        let mut p = Project::default();
        for mat in 1u8..=4 {
            add_cube(&mut p, mat);
        }
        // Override the auto-bound M4 → AMS:4: pin to Ext (slot 4 in
        // bambi's AMS-first layout).
        p.plates[0].material_to_slot.insert(
            4,
            SlotRef {
                extruder: 0,
                slot: 4,
            },
        );
        let (use_ams, mapping, mapping2) = ams_mapping_for_plate(&p.plates[0]);
        assert!(use_ams);
        assert_eq!(mapping, vec![0, 1, 2, -1]);
        assert_eq!(
            mapping2[0],
            AmsMappingV2 {
                ams_id: 0,
                slot_id: 0
            }
        );
        assert_eq!(
            mapping2[1],
            AmsMappingV2 {
                ams_id: 0,
                slot_id: 1
            }
        );
        assert_eq!(
            mapping2[2],
            AmsMappingV2 {
                ams_id: 0,
                slot_id: 2
            }
        );
        assert_eq!(
            mapping2[3],
            AmsMappingV2 {
                ams_id: 255,
                slot_id: 0
            }
        );
    }

    #[test]
    fn ams_mapping_with_ext_at_first_material_preserves_position() {
        // 2-material print, M1 on Ext, M2 on AMS:1. Exact match to
        // BBS's captured 2-mat shape:
        //   ams_mapping  = [-1, 0]
        //   ams_mapping2 = [{255,0}, {0,0}]
        // Indexing by material position (NOT slot position) keeps
        // the Ext sentinel at filament index 0 where M1 lives.
        use crate::core::printer::SlotRef;
        let _registry = RegistryGuard::acquire();
        let mut p = Project::default();
        for mat in 1u8..=2 {
            add_cube(&mut p, mat);
        }
        // Explicit pins — auto-bind order varies with insertion
        // sequence; force the bindings the test cares about.
        p.plates[0].material_to_slot.insert(
            1,
            SlotRef {
                extruder: 0,
                slot: 4,
            },
        ); // Ext
        p.plates[0].material_to_slot.insert(
            2,
            SlotRef {
                extruder: 0,
                slot: 0,
            },
        ); // AMS:1
        let (use_ams, mapping, mapping2) = ams_mapping_for_plate(&p.plates[0]);
        assert!(use_ams);
        assert_eq!(mapping, vec![-1, 0]);
        assert_eq!(
            mapping2[0],
            AmsMappingV2 {
                ams_id: 255,
                slot_id: 0
            }
        );
        assert_eq!(
            mapping2[1],
            AmsMappingV2 {
                ams_id: 0,
                slot_id: 0
            }
        );
    }

    #[test]
    fn ams_mapping_external_only_emits_ext_sentinel_and_use_ams_false() {
        // Single-material print, M1 pinned to Ext (slot 4 in
        // bambi's AMS-first layout). `mapping[0] = -1` and
        // `mapping2[0] = {255, 0}` (Ext sentinel, NOT the unused
        // `{255, 255}` — the firmware reads the difference to
        // route to the external spool). `use_ams` is false because
        // no material lands on an AMS slot.
        use crate::core::printer::SlotRef;
        let _registry = RegistryGuard::acquire();
        let mut p = Project::default();
        add_cube(&mut p, 1);
        p.plates[0].material_to_slot.insert(
            1,
            SlotRef {
                extruder: 0,
                slot: 4,
            },
        );
        let (use_ams, mapping, mapping2) = ams_mapping_for_plate(&p.plates[0]);
        assert!(!use_ams);
        assert_eq!(mapping, vec![-1]);
        assert_eq!(
            mapping2,
            vec![AmsMappingV2 {
                ams_id: 255,
                slot_id: 0
            }]
        );
    }

    #[test]
    fn ams_bindings_empty_for_snappy_toolchanger() {
        // Snappy is a 4-toolhead toolchanger — all slots are
        // Direct-feed, not AMS. ams_bindings is a BBL-firmware
        // concept and shouldn't carry entries for a U1 plate
        // (the U1 send path doesn't even wrap as .gcode.3mf, but
        // the encoder should still produce a clean empty output
        // for hygiene).
        let _registry = RegistryGuard::acquire();
        let mut p = Project::default();
        p.plates[0].printer_instance_id = Some("snappy".into());
        for mat in 1u8..=4 {
            add_cube(&mut p, mat);
        }
        let bindings = ams_bindings_for_plate(&p.plates[0]);
        assert!(bindings.is_empty(), "got {bindings:?}");
    }
}
