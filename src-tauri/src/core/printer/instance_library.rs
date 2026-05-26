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
    BedRef, ExtruderState, FeedKind, NozzleMaterial, NozzleSku, PrinterInstance, SlotBinding,
};

/// Stable IDs for the bundled instances. Used as
/// `Plate.printer_instance_id`.
pub const BAMBI_ID: &str = "bambi";
pub const SNAPPY_ID: &str = "snappy";

/// Seed for the mutable in-memory registry — built fresh every call
/// so callers can produce a clean reset list. Insertion order is the
/// picker's display order. Once the registry initializes from this,
/// subsequent `lookup_instance` calls read live (possibly user-
/// mutated) state.
pub fn bundled_instances() -> Vec<PrinterInstance> {
    vec![bambi(), snappy()]
}

/// Bridge from a legacy `PrinterBinding.printer_identity` (vendor
/// profile slug like `"bambu-lab-a1-mini"`) to the matching bundled
/// PrinterInstance id (`"bambi"`). Returns `None` for vendor profiles
/// that don't yet have a corresponding bundled instance fixture.
///
/// Lives here (not on `PrinterInstance` itself) because the bundled
/// set is the authoritative mapping — PrinterInstances reference
/// vendor profiles by name, and this function reverses that lookup.
/// Used by plate bootstrap + printer rebinding to keep the legacy
/// `Plate.printer` field in sync with the new `Plate.printer_instance_id`.
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
        vendor_profile_ref: "bambu-lab-a1-mini".to_owned(),
        // PR-S-4 rework: slug is the printer model, NOT the per-nozzle
        // variant. The nozzle SKU lives on the extruder state below;
        // the composer loads `printer/<slug>/nozzles/<sku>.toml`.
        printer_fragment_slug: "bambu-lab-a1-mini".to_owned(),
        default_filament_fragment_slug: "bambu-pla-basic-bbl-a1m".to_owned(),
        default_process_fragment_slug: "0.20mm-standard-bbl-a1m".to_owned(),
        connection: None,
        extruders: vec![ExtruderState {
            // Solo extruder — slot labels carry the full identity.
            label: String::new(),
            installed_nozzle: NozzleSku {
                diameter_mm: 0.4,
                material: NozzleMaterial::Stainless,
            },
            // A1 mini + AMS Lite: 4 `Ams`-feed slots followed by 1
            // direct-fed `Ext` slot. The pre-slice gate refuses
            // prints that mix `Ext` with any `AMS:n` slot in the
            // same job (Bambu firmware physically can't pull from
            // both feed paths). Multiple `AMS:n` slots are fine —
            // the AMS swaps within.
            //
            // AMS-first / Ext-last is cosmetic: the binding panel
            // and the slot-color strip read more naturally when
            // the AMS row is the dominant fixture and the external
            // spool sits at the end. Slot order has no semantic
            // meaning to libslic3r or the firmware — the
            // material→filament mapping is built from the user's
            // material bindings, not from slot position.
            //
            // Seeded with the colors of the spools physically loaded
            // in this fixture's printer right now. Once printer-instance
            // editing or driver-side AMS sync land these become the
            // initial defaults rather than hardcoded values.
            slots: vec![
                SlotBinding {
                    label: "AMS:1".to_owned(),
                    feed: FeedKind::Ams,
                    filament_identity: Some("generic-pla".to_owned()),
                    color: Some("#111827".to_owned()),
                },
                SlotBinding {
                    label: "AMS:2".to_owned(),
                    feed: FeedKind::Ams,
                    filament_identity: Some("generic-pla".to_owned()),
                    color: Some("#d4a017".to_owned()),
                },
                SlotBinding {
                    label: "AMS:3".to_owned(),
                    feed: FeedKind::Ams,
                    filament_identity: Some("generic-pla".to_owned()),
                    color: Some("#5b21b6".to_owned()),
                },
                SlotBinding {
                    label: "AMS:4".to_owned(),
                    feed: FeedKind::Ams,
                    filament_identity: Some("generic-pla".to_owned()),
                    color: Some("#ea580c".to_owned()),
                },
                SlotBinding {
                    label: "Ext".to_owned(),
                    feed: FeedKind::Direct,
                    filament_identity: Some("generic-pla".to_owned()),
                    color: Some("#dc2626".to_owned()),
                },
            ],
        }],
        bed: BedRef {
            identity: "Supertack Plate".to_owned(),
        },
        config_overrides: Default::default(),
    }
}

fn snappy() -> PrinterInstance {
    // Per-extruder direct feed. Each extruder is independent; no AMS
    // in the topology so the feed-mixing gate is trivially satisfied
    // (one slot per extruder). Seeded colors are the physical spools
    // currently loaded in this fixture's printer.
    let extruder = |label: &str, color: &str| ExtruderState {
        label: label.to_owned(),
        installed_nozzle: NozzleSku {
            diameter_mm: 0.4,
            material: NozzleMaterial::Stainless,
        },
        slots: vec![SlotBinding {
            label: String::new(),
            feed: FeedKind::Direct,
            filament_identity: Some("generic-pla".to_owned()),
            color: Some(color.to_owned()),
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
        extruders: vec![
            extruder("T0", "#dc2626"),
            extruder("T1", "#eab308"),
            extruder("T2", "#111827"),
            extruder("T3", "#f8fafc"),
        ],
        bed: BedRef {
            identity: "Textured PEI Plate".to_owned(),
        },
        config_overrides: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::instance_registry::RegistryGuard;
    use crate::core::printer::lookup_instance;

    #[test]
    fn bundled_set_is_bambi_then_snappy() {
        let _registry = RegistryGuard::acquire();
        let instances = bundled_instances();
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].id, BAMBI_ID);
        assert_eq!(instances[0].display_name, "Bambi");
        assert_eq!(instances[1].id, SNAPPY_ID);
        assert_eq!(instances[1].display_name, "Snappy");
    }

    #[test]
    fn bambi_is_a1_mini_with_ams_lite_supertack_and_stainless() {
        use crate::core::printer::FeedKind;

        let _registry = RegistryGuard::acquire();
        let b = lookup_instance(BAMBI_ID).expect("bambi present");
        assert_eq!(b.vendor_profile_ref, "bambu-lab-a1-mini");
        assert_eq!(b.extruders.len(), 1);
        assert_eq!(b.extruders[0].installed_nozzle.diameter_mm, 0.4);
        assert_eq!(
            b.extruders[0].installed_nozzle.material,
            NozzleMaterial::Stainless,
        );
        // A1 mini + AMS Lite: 5 slots — 4 Ams (`AMS:1`..`AMS:4`)
        // followed by 1 Direct (`Ext`). AMS-first is cosmetic; see
        // the comment in `bambi()`.
        let slots = &b.extruders[0].slots;
        assert_eq!(slots.len(), 5);
        let labels: Vec<&str> = slots.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["AMS:1", "AMS:2", "AMS:3", "AMS:4", "Ext"]);
        for ams in &slots[..4] {
            assert_eq!(ams.feed, FeedKind::Ams);
        }
        assert_eq!(slots[4].feed, FeedKind::Direct);
        assert_eq!(b.bed.identity, "Supertack Plate");
    }

    #[test]
    fn snappy_is_u1_with_four_independent_extruders_and_textured_pei() {
        let _registry = RegistryGuard::acquire();
        let s = lookup_instance(SNAPPY_ID).expect("snappy present");
        assert_eq!(s.vendor_profile_ref, "snapmaker-u1");
        assert_eq!(s.extruders.len(), 4);
        for extruder in &s.extruders {
            assert_eq!(extruder.slots.len(), 1);
            assert_eq!(extruder.installed_nozzle.diameter_mm, 0.4);
            assert_eq!(extruder.installed_nozzle.material, NozzleMaterial::Stainless);
        }
        assert_eq!(s.bed.identity, "Textured PEI Plate");
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
