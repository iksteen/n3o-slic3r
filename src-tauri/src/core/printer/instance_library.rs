//! Bundled printer instances (PR-S-3).
//!
//! MVP hardcodes two physical printers the developer/test rigs use:
//!
//! - **Bambi** — Bambu A1 mini, 0.4mm stainless steel nozzle, Cool Plate
//!   SuperTack bed. Single extruder, single slot (no AMS in this fixture).
//! - **Snappy** — Snapmaker U1, 4 extruders, each with a 0.4mm stainless
//!   steel nozzle and one slot. Textured PEI bed.
//!
//! These exist so the picker has something to point at while the user-
//! library + add-instance UI is unbuilt. Both instances reference vendor
//! profiles via [`crate::core::printer::bundled_catalog`] — the same
//! [`PrinterProfile`]s the existing PrinterPicker has been using.
//!
//! Slot bindings start unbound (`filament_identity: None`); the
//! slice-input builder falls back to "Generic PLA" for unbound slots
//! until a filament picker lands.

use super::instance::{
    BedRef, ExtruderState, NozzleMaterial, NozzleSku, PrinterInstance, SlotBinding,
};

/// Stable IDs for the bundled instances. Used as
/// `Plate.printer_instance_ref` once that migration lands.
pub const BAMBI_ID: &str = "bambi";
pub const SNAPPY_ID: &str = "snappy";

/// Bundled instance set — exactly two for MVP. Insertion order is also
/// the picker's display order.
pub fn bundled_instances() -> Vec<PrinterInstance> {
    vec![bambi(), snappy()]
}

/// Look up a bundled instance by id. Returns `None` for unknown ids.
pub fn lookup_instance(id: &str) -> Option<PrinterInstance> {
    bundled_instances().into_iter().find(|i| i.id == id)
}

/// Bridge from a legacy `PrinterBinding.printer_identity` (vendor
/// profile slug like `"bambu-a1-mini"`) to the matching bundled
/// PrinterInstance id (`"bambi"`). Returns `None` for vendor profiles
/// that don't yet have a corresponding bundled instance fixture.
///
/// Lives here (not on `PrinterInstance` itself) because the bundled
/// set is the authoritative mapping — PrinterInstances reference
/// vendor profiles by name, and this function reverses that lookup.
/// Used by plate bootstrap + printer rebinding to keep the legacy
/// `Plate.printer` field in sync with the new `Plate.printer_instance_id`
/// through the PR-S-5c transition.
pub fn instance_id_for_vendor_profile(vendor_profile_ref: &str) -> Option<&'static str> {
    bundled_instances()
        .iter()
        .find(|i| i.vendor_profile_ref == vendor_profile_ref)
        .map(|i| match i.id.as_str() {
            "bambi" => BAMBI_ID,
            "snappy" => SNAPPY_ID,
            _ => unreachable!(),
        })
}

fn bambi() -> PrinterInstance {
    PrinterInstance {
        id: BAMBI_ID.to_owned(),
        display_name: "Bambi".to_owned(),
        vendor_profile_ref: "bambu-a1-mini".to_owned(),
        // PR-S-4 rework: slug is the printer model, NOT the per-nozzle
        // variant. The nozzle SKU lives on the extruder state below;
        // the composer loads `printer/<slug>/nozzles/<sku>.toml`.
        printer_fragment_slug: "bambu-lab-a1-mini".to_owned(),
        default_filament_fragment_slug: "bambu-pla-basic-bbl-a1m".to_owned(),
        default_process_fragment_slug: "0.20mm-standard-bbl-a1m".to_owned(),
        connection: None,
        extruders: vec![ExtruderState {
            installed_nozzle: NozzleSku {
                diameter_mm: 0.4,
                material: NozzleMaterial::Stainless,
            },
            // Single slot — the A1 mini direct feed. Adding AMS Lite
            // later would extend this to 5 slots (1 direct + 4 AMS).
            slots: vec![SlotBinding {
                filament_identity: None,
            }],
        }],
        bed: BedRef {
            identity: "Bambu Cool Plate SuperTack".to_owned(),
        },
        config_overrides: Default::default(),
    }
}

fn snappy() -> PrinterInstance {
    let extruder = || ExtruderState {
        installed_nozzle: NozzleSku {
            diameter_mm: 0.4,
            material: NozzleMaterial::Stainless,
        },
        slots: vec![SlotBinding {
            filament_identity: None,
        }],
    };
    PrinterInstance {
        id: SNAPPY_ID.to_owned(),
        display_name: "Snappy".to_owned(),
        vendor_profile_ref: "snapmaker-u1".to_owned(),
        printer_fragment_slug: "snapmaker-u1".to_owned(),
        default_filament_fragment_slug: "snapmaker-pla-u1".to_owned(),
        default_process_fragment_slug: "0.20-standard-snapmaker-u1-0.4-nozzle".to_owned(),
        connection: None,
        extruders: vec![extruder(), extruder(), extruder(), extruder()],
        bed: BedRef {
            identity: "Snapmaker Textured PEI".to_owned(),
        },
        config_overrides: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_set_is_bambi_then_snappy() {
        let instances = bundled_instances();
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].id, BAMBI_ID);
        assert_eq!(instances[0].display_name, "Bambi");
        assert_eq!(instances[1].id, SNAPPY_ID);
        assert_eq!(instances[1].display_name, "Snappy");
    }

    #[test]
    fn bambi_is_a1_mini_with_supertack_and_stainless() {
        let b = lookup_instance(BAMBI_ID).expect("bambi present");
        assert_eq!(b.vendor_profile_ref, "bambu-a1-mini");
        assert_eq!(b.extruders.len(), 1);
        assert_eq!(b.extruders[0].slots.len(), 1);
        assert_eq!(b.extruders[0].installed_nozzle.diameter_mm, 0.4);
        assert_eq!(
            b.extruders[0].installed_nozzle.material,
            NozzleMaterial::Stainless,
        );
        assert_eq!(b.bed.identity, "Bambu Cool Plate SuperTack");
    }

    #[test]
    fn snappy_is_u1_with_four_independent_extruders_and_textured_pei() {
        let s = lookup_instance(SNAPPY_ID).expect("snappy present");
        assert_eq!(s.vendor_profile_ref, "snapmaker-u1");
        assert_eq!(s.extruders.len(), 4);
        for extruder in &s.extruders {
            assert_eq!(extruder.slots.len(), 1);
            assert_eq!(extruder.installed_nozzle.diameter_mm, 0.4);
            assert_eq!(extruder.installed_nozzle.material, NozzleMaterial::Stainless);
        }
        assert_eq!(s.bed.identity, "Snapmaker Textured PEI");
    }

    #[test]
    fn vendor_profile_refs_resolve_against_bundled_catalog() {
        // Sanity: the instances point at profiles that actually exist
        // in the printer catalog.
        let catalog = crate::core::printer::bundled_catalog();
        for inst in bundled_instances() {
            assert!(
                catalog
                    .iter()
                    .any(|e| e.identity == inst.vendor_profile_ref),
                "{} references missing vendor profile {}",
                inst.id,
                inst.vendor_profile_ref,
            );
        }
    }

    #[test]
    fn lookup_unknown_id_is_none() {
        assert!(lookup_instance("ghost-printer").is_none());
    }
}
