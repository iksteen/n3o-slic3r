//! Bundled printer instances — **test-only fixtures.**
//!
//! Production never seeds from these. The on-disk user library
//! (`instance_storage`) starts empty on first launch and the
//! add-printer wizard writes the first instance. `bundled_instances()`
//! is only reachable from tests that don't call `init_root`, where
//! `instance_registry` falls back to this in-memory set so the wide
//! test surface doesn't need temp-library plumbing per test.
//!
//! Three fixtures spanning the routing shapes a test might need. In
//! production the user-library path adds printers; here they're just enough
//! to exercise AMS, firmware-routed toolchanger, and plain single-extruder
//! paths without temp-library plumbing.
//!
//! - **Bambi** — Bambu A1 mini, 0.4mm stainless steel nozzle, Cool Plate
//!   SuperTack bed. Single extruder × 5 slots (4 AMS + 1 Ext).
//! - **Snappy** — Snapmaker U1, 4 extruders × 1 slot each, 0.4mm
//!   stainless steel nozzle per toolhead. Textured PEI bed. Firmware-routed
//!   (`driver_kind = U1`) → the per-material MAP_TABLE path, not slot-fan.
//! - **Bender** — Creality Ender-3 S1 (Klipper): the plain baseline. One
//!   extruder × one Direct slot, no AMS, not a toolchanger, not
//!   firmware-routed. The "just needs a bound printer" fixture, free of
//!   AMS / U1 routing special-casing.

use super::instance::{
    BedRef, ExtruderState, FeedKind, NozzleMaterial, NozzleSku, PrinterInstance, SlotBinding,
};

/// Stable IDs for the test-fixture instances. Tests pin these as
/// `Plate.printer_instance_id` when they want to address bambi /
/// snappy directly without going through whatever the on-disk
/// library happens to hold.
pub const BAMBI_ID: &str = "bambi";
pub const SNAPPY_ID: &str = "snappy";
pub const BENDER_ID: &str = "bender";

/// In-memory fixture set the registry falls back to when no storage
/// root has been registered — i.e. in tests. Built fresh every call
/// so test setups can produce a clean reset list. Bambi stays first so
/// `with_preferred_printer(None)` keeps binding it as the default.
pub fn bundled_instances() -> Vec<PrinterInstance> {
    vec![bambi(), snappy(), bender()]
}

/// Reverse lookup: given a vendor profile ref (e.g.
/// `"bambu-lab-a1-mini"`), return the id of the matching test-fixture
/// `PrinterInstance` (`"bambi"`). `None` for vendor profiles with no
/// fixture. Tests use this to bridge a `PrinterBinding`-shaped
/// identity back to a fixture id without hardcoding the mapping at
/// every call site.
///
/// Lives here because the fixture set is the authoritative mapping —
/// instances reference vendor profiles by name, and this is the
/// inverse.
pub fn instance_id_for_vendor_profile(vendor_profile_ref: &str) -> Option<&'static str> {
    bundled_instances()
        .iter()
        .find(|i| i.vendor_profile_ref == vendor_profile_ref)
        .map(|i| match i.id.as_str() {
            "bambi" => BAMBI_ID,
            "snappy" => SNAPPY_ID,
            "bender" => BENDER_ID,
            _ => unreachable!(),
        })
}

fn bambi() -> PrinterInstance {
    PrinterInstance {
        id: BAMBI_ID.to_owned(),
        display_name: "Bambi".to_owned(),
        vendor_profile_ref: "bambu-lab-a1-mini".to_owned(),
        // Slug is the printer model, NOT the per-nozzle variant.
        // The nozzle SKU lives on the extruder state below; the
        // composer loads `printer/<slug>/nozzles/<sku>.toml`.
        printer_fragment_slug: "bambu-lab-a1-mini".to_owned(),
        default_filament_fragment_slug: "bambu-pla-basic".to_owned(),
        quality_profile: "0.20mm-standard".to_owned(),
        connection: None,
        extruders: vec![ExtruderState {
            // Solo extruder — slot labels carry the full identity.
            installed_nozzle: NozzleSku {
                diameter: "0.4".to_string(),
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
                    feed: FeedKind::Ams,
                    filament_identity: Some("generic-pla".to_owned()),
                    color: Some("#111827".to_owned()),
                    tag_uid: None,
                },
                SlotBinding {
                    feed: FeedKind::Ams,
                    filament_identity: Some("generic-pla".to_owned()),
                    color: Some("#d4a017".to_owned()),
                    tag_uid: None,
                },
                SlotBinding {
                    feed: FeedKind::Ams,
                    filament_identity: Some("generic-pla".to_owned()),
                    color: Some("#5b21b6".to_owned()),
                    tag_uid: None,
                },
                SlotBinding {
                    feed: FeedKind::Ams,
                    filament_identity: Some("generic-pla".to_owned()),
                    color: Some("#ea580c".to_owned()),
                    tag_uid: None,
                },
                SlotBinding {
                    feed: FeedKind::Direct,
                    filament_identity: Some("generic-pla".to_owned()),
                    color: Some("#dc2626".to_owned()),
                    tag_uid: None,
                },
            ],
        }],
        bed: BedRef {
            identity: "Supertack Plate".to_owned(),
        },
        config_overrides: Default::default(),
        send_options: Default::default(),
    }
}

fn snappy() -> PrinterInstance {
    // Per-extruder direct feed. Each extruder is independent; no AMS
    // in the topology so the feed-mixing gate is trivially satisfied
    // (one slot per extruder). Seeded colors are the physical spools
    // currently loaded in this fixture's printer.
    let extruder = |color: &str| ExtruderState {
        installed_nozzle: NozzleSku {
            diameter: "0.4".to_string(),
            material: NozzleMaterial::Stainless,
        },
        slots: vec![SlotBinding {
            feed: FeedKind::Direct,
            filament_identity: Some("generic-pla".to_owned()),
            color: Some(color.to_owned()),
            tag_uid: None,
        }],
    };
    PrinterInstance {
        id: SNAPPY_ID.to_owned(),
        display_name: "Snappy".to_owned(),
        vendor_profile_ref: "snapmaker-u1".to_owned(),
        printer_fragment_slug: "snapmaker-u1".to_owned(),
        default_filament_fragment_slug: "snapmaker-pla".to_owned(),
        quality_profile: "0.20-standard".to_owned(),
        connection: None,
        extruders: vec![
            extruder("#dc2626"),
            extruder("#eab308"),
            extruder("#111827"),
            extruder("#f8fafc"),
        ],
        bed: BedRef {
            identity: "Textured PEI Plate".to_owned(),
        },
        config_overrides: Default::default(),
        send_options: Default::default(),
    }
}

fn bender() -> PrinterInstance {
    // Ender-3 S1 Klipper: one Direct-fed extruder, no AMS. `driver_kind` is
    // Moonraker (not U1), so it's neither firmware-routed nor a toolchanger —
    // multi-material collapses onto the single toolhead (all `T0`).
    PrinterInstance {
        id: BENDER_ID.to_owned(),
        display_name: "Bender".to_owned(),
        vendor_profile_ref: "creality-ender-3-s1-klipper".to_owned(),
        printer_fragment_slug: "creality-ender-3-s1-klipper".to_owned(),
        default_filament_fragment_slug: "generic-pla".to_owned(),
        quality_profile: "0.20mm-standard".to_owned(),
        connection: None,
        extruders: vec![ExtruderState {
            installed_nozzle: NozzleSku {
                diameter: "0.4".to_string(),
                material: NozzleMaterial::Brass,
            },
            slots: vec![SlotBinding {
                feed: FeedKind::Direct,
                filament_identity: Some("generic-pla".to_owned()),
                color: Some("#22c55e".to_owned()),
                tag_uid: None,
            }],
        }],
        bed: BedRef {
            identity: "Textured PEI Plate".to_owned(),
        },
        config_overrides: Default::default(),
        send_options: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::instance_registry::RegistryGuard;
    use crate::core::printer::lookup_instance;

    #[test]
    fn bundled_set_is_bambi_snappy_bender() {
        let _registry = RegistryGuard::acquire();
        let instances = bundled_instances();
        assert_eq!(instances.len(), 3);
        assert_eq!(instances[0].id, BAMBI_ID);
        assert_eq!(instances[0].display_name, "Bambi");
        assert_eq!(instances[1].id, SNAPPY_ID);
        assert_eq!(instances[1].display_name, "Snappy");
        assert_eq!(instances[2].id, BENDER_ID);
        assert_eq!(instances[2].display_name, "Bender");
    }

    #[test]
    fn bender_is_single_extruder_no_ams_not_firmware_routed() {
        use crate::core::printer::FeedKind;
        let _registry = RegistryGuard::acquire();
        let b = lookup_instance(BENDER_ID).expect("bender present");
        assert_eq!(b.vendor_profile_ref, "creality-ender-3-s1-klipper");
        // One extruder, one Direct slot — no AMS, not a toolchanger.
        assert_eq!(b.extruders.len(), 1);
        assert_eq!(b.extruders[0].slots.len(), 1);
        assert_eq!(b.extruders[0].slots[0].feed, FeedKind::Direct);
        assert_eq!(b.bed.identity, "Textured PEI Plate");
    }

    #[test]
    fn bambi_is_a1_mini_with_ams_lite_supertack_and_stainless() {
        use crate::core::printer::FeedKind;

        let _registry = RegistryGuard::acquire();
        let b = lookup_instance(BAMBI_ID).expect("bambi present");
        assert_eq!(b.vendor_profile_ref, "bambu-lab-a1-mini");
        assert_eq!(b.extruders.len(), 1);
        assert_eq!(b.extruders[0].installed_nozzle.diameter, "0.4");
        assert_eq!(
            b.extruders[0].installed_nozzle.material,
            NozzleMaterial::Stainless,
        );
        // A1 mini + AMS Lite: 5 slots — 4 Ams followed by 1 Direct.
        // AMS-first is cosmetic; see the comment in `bambi()`. Slot
        // display labels live in the frontend, not on these
        // structs.
        let slots = &b.extruders[0].slots;
        assert_eq!(slots.len(), 5);
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
            assert_eq!(extruder.installed_nozzle.diameter, "0.4");
            assert_eq!(
                extruder.installed_nozzle.material,
                NozzleMaterial::Stainless
            );
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
