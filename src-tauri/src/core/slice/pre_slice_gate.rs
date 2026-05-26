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
    use crate::core::printer::FeedKind;
    let Some(instance_id) = plate.printer_instance_id.as_deref() else {
        return Vec::new();
    };
    let Some(instance) = lookup_instance(instance_id) else {
        return Vec::new();
    };

    // BBL firmware expects 1-based AMS-only slot indices (1..N for an
    // N-slot AMS unit) in the `ams_bindings.ams_slot` field — these
    // become the `M620 S<N>A` operands the print pre-loads. Direct-
    // fed slots (the A1 mini's external spool) aren't AMS-addressable;
    // material loading from a Direct slot is signaled via the
    // separate `ams_mapping` field's `-1` sentinel and shouldn't
    // appear in ams_bindings at all.
    //
    // Earlier implementation walked the *full* flat slot grid
    // (including Direct slots) with 1-based numbering — Bambi's
    // `[Ext, AMS:1..4]` got `[1, 2, 3, 4, 5]`, so material auto-bound
    // to AMS:1 published ams_slot=2 and material auto-bound to AMS:4
    // published ams_slot=5 (which doesn't exist on the 4-slot AMS
    // lite and produced a firmware-side "filament loading error" on a
    // real 4-color print). Now mirror `ams_mapping_for_plate`: skip
    // Direct-feed slots and number AMS-feed slots within their
    // extruder, 1-based.
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
    pub const UNUSED: Self = Self { ams_id: 255, slot_id: 255 };
}

/// Compute the Bambu MQTT `project_file` AMS routing fields for a
/// plate: `(use_ams, ams_mapping[5], ams_mapping2[5])`.
///
/// Both arrays are 5-element fixed-length, left-aligned (filament 0
/// at index 0). `ams_mapping[i]` is the 0-based flat AMS slot id
/// (0..3) for filament i, `-1` for external; `ams_mapping2[i]` is
/// the structured `{ams_id, slot_id}` pair (`{255, 255}` for
/// unused). Both shapes are required — the firmware on A1 mini
/// firmware needs them in tandem, and the BBS publish always sends
/// both. `use_ams` is true when at least one referenced material
/// lands on an `Ams`-feed slot.
///
/// MVP: we assume a single AMS unit per extruder (the A1 mini + AMS
/// Lite case), so `ams_id` is always `0` for AMS-fed slots. Multi-
/// AMS printers (X1C with 4 AMS units) need a richer printer-side
/// model to know which AMS each `Ams`-feed slot belongs to.
pub fn ams_mapping_for_plate(
    plate: &crate::core::project::model::Plate,
) -> (bool, [i8; 5], [AmsMappingV2; 5]) {
    use crate::core::printer::FeedKind;
    let mut mapping = [-1i8; 5];
    let mut mapping2 = [AmsMappingV2::UNUSED; 5];
    let Some(instance_id) = plate.printer_instance_id.as_deref() else {
        return (false, mapping, mapping2);
    };
    let Some(instance) = lookup_instance(instance_id) else {
        return (false, mapping, mapping2);
    };

    // `ams_mapping[i]` is the AMS slot for **libslic3r filament index
    // i** (0-based), NOT for the i-th entry in `material_to_slot`
    // iteration order. The cascade composer fans out one filament per
    // PrinterInstance slot (in flat extruder-major order); the slicer
    // emits `T<n>` referring to those filament indices; the firmware
    // looks up `ams_mapping[n]` to find which physical AMS slot to
    // pull from. Indexing by iteration position instead of filament
    // index produced an off-by-one (or off-by-N when Direct slots
    // exist) — on Bambi (`[Ext, AMS:1..AMS:4]`) we sent
    // `[0, 1, 2, 3, -1]` which the firmware read as
    // "filament 0 → AMS:1, filament 1 → AMS:2, …, filament 4 →
    // external", shifting every cube's color one slot up and
    // producing a "no AMS slot 5" error on the 4th color.
    //
    // Correct indexing:
    //   filament_index = sum(prev extruders' slot counts) + slot_ref.slot
    //   ams_slot_in_unit = count(AMS-feed slots in this extruder, [..slot])
    let mut any_ams = false;
    for slot_ref in plate.material_to_slot.values() {
        let Some(ext) = instance.extruders.get(slot_ref.extruder as usize) else {
            continue;
        };
        let Some(slot) = ext.slots.get(slot_ref.slot as usize) else {
            continue;
        };
        if slot.feed != FeedKind::Ams {
            continue;
        }
        let preceding: usize = instance
            .extruders
            .iter()
            .take(slot_ref.extruder as usize)
            .map(|e| e.slots.len())
            .sum();
        let filament_index = preceding + slot_ref.slot as usize;
        if filament_index >= mapping.len() {
            continue;
        }
        let ams_slot = ext.slots[..slot_ref.slot as usize]
            .iter()
            .filter(|s| s.feed == FeedKind::Ams)
            .count() as u8;
        mapping[filament_index] = ams_slot as i8;
        mapping2[filament_index] = AmsMappingV2 { ams_id: 0, slot_id: ams_slot };
        any_ams = true;
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
    use crate::core::printer::instance_registry::RegistryGuard;
    use crate::core::printer::set_slot_filament;
    use crate::core::printer::SlotRef;
    use crate::core::scene::state::{MeshProvenance, NewMesh, NewSceneObject};
    use crate::core::scene::transform::Transform;
    use crate::core::printer::profile::BoundingBox;

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
        assert!(err.issues.iter().any(|i| matches!(i, SliceBlocker::UnboundPrinter)));
    }

    #[test]
    fn slot_without_filament_blocks() {
        let _registry = RegistryGuard::acquire();
        // Bundled fixtures now ship every slot pre-bound to
        // `generic-pla`. Auto-bind on Bambi skips the external spool
        // (slot 0) and lands material 1 on AMS:1 (slot 1) — clear
        // that slot's filament so the gate sees an unbound slot in
        // the material's path.
        set_slot_filament("bambi", 0, 1, None).unwrap();
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
        p.plates[0]
            .material_to_slot
            .insert(1, SlotRef { extruder: 5, slot: 0 });
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
        // Bambi has 1 extruder × 5 slots (Ext + AMS:1..AMS:4). Auto-
        // bind skips the external spool when AMS slots exist, so
        // material 1 lands on (0, 1) = AMS:1 → ams_slot=1 (first
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
        // (Direct-feed slot at index 0) should NOT publish an
        // ams_bindings entry — the firmware reads "use external
        // spool" via the separate `ams_mapping` field's `-1`
        // sentinel. Publishing it as an AMS slot would route the
        // print to a real AMS slot the user didn't choose.
        use crate::core::printer::SlotRef;
        let _registry = RegistryGuard::acquire();
        let mut p = Project::default();
        // Override the auto-bind: pin M1 to Ext.
        add_cube(&mut p, 1);
        p.plates[0]
            .material_to_slot
            .insert(1, SlotRef { extruder: 0, slot: 0 });
        let bindings = ams_bindings_for_plate(&p.plates[0]);
        assert!(bindings.is_empty(), "got {bindings:?}");
    }

    #[test]
    fn ams_mapping_for_4mat_bambi_indexed_by_filament_not_material_position() {
        // Regression for the wrong-colors symptom the user hit on a
        // real 4-cube print: every cube printed with the next slot's
        // filament, and the 4th errored as "filament loading
        // failed". The encoder had been indexing `mapping[i]` by the
        // position of the material in the BTreeMap iteration order
        // (0..3), but the firmware indexes it by libslic3r FILAMENT
        // INDEX (0..N where N = sum of all PrinterInstance slot
        // counts). Bambi's `[Ext, AMS:1..AMS:4]` means filament 0 =
        // Ext (no AMS), filaments 1..4 = AMS:1..AMS:4 — so the
        // correct shape is `[-1, 0, 1, 2, 3]` (Ext at index 0),
        // not `[0, 1, 2, 3, -1]`.
        let _registry = RegistryGuard::acquire();
        let mut p = Project::default();
        for mat in 1u8..=4 {
            add_cube(&mut p, mat);
        }
        let (use_ams, mapping, mapping2) = ams_mapping_for_plate(&p.plates[0]);
        assert!(use_ams);
        // Material 1 → AMS:1 → filament index 1 → AMS slot 0 (0-based,
        // AMS-feed-only). Material 4 → AMS:4 → filament index 4 →
        // AMS slot 3. Filament 0 (Ext) is unused → -1.
        assert_eq!(mapping, [-1, 0, 1, 2, 3]);
        assert_eq!(mapping2[0], AmsMappingV2::UNUSED);
        assert_eq!(mapping2[1], AmsMappingV2 { ams_id: 0, slot_id: 0 });
        assert_eq!(mapping2[2], AmsMappingV2 { ams_id: 0, slot_id: 1 });
        assert_eq!(mapping2[3], AmsMappingV2 { ams_id: 0, slot_id: 2 });
        assert_eq!(mapping2[4], AmsMappingV2 { ams_id: 0, slot_id: 3 });
    }

    #[test]
    fn ams_mapping_external_only_routes_to_ext_filament_index_with_unused() {
        // Material pinned manually to the Bambi external spool (Ext,
        // flat slot 0 / filament 0). No AMS-feed slot is referenced,
        // so `mapping` stays all-`-1`, `use_ams = false`.
        use crate::core::printer::SlotRef;
        let _registry = RegistryGuard::acquire();
        let mut p = Project::default();
        add_cube(&mut p, 1);
        p.plates[0]
            .material_to_slot
            .insert(1, SlotRef { extruder: 0, slot: 0 });
        let (use_ams, mapping, mapping2) = ams_mapping_for_plate(&p.plates[0]);
        assert!(!use_ams);
        assert_eq!(mapping, [-1, -1, -1, -1, -1]);
        assert!(mapping2.iter().all(|m| *m == AmsMappingV2::UNUSED));
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
