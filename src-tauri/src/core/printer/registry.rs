//! Printer profile registry.
//!
//! Thin facade over [`crate::core::profile_library::printer_catalog`].
//! The library walks the vendor profile tree at startup and parses one
//! `model.toml` per printer directory; this module re-exposes that
//! data in the picker-facing shape (`CatalogEntry`).
//!
//! Surface:
//! - [`bundled_catalog()`] — every entry from the library, in
//!   declaration order (vendor sort order, then printer directory
//!   sort order within each vendor).
//! - [`lookup(identity)`] — find a profile by its identity slug.
//! - [`CatalogEntry`] — the small summary the picker UI consumes
//!   without having to round-trip the full profile.

use serde::{Deserialize, Serialize};

use super::profile::PrinterProfile;
use crate::core::profile_library;

/// Picker-facing entry for one printer in the catalog. Carries the
/// identity slug + the full `PrinterProfile`. The picker chip + menu
/// only read `identity`, `profile.model`, and
/// `profile.supported_build_plates`, but the rest of the panel
/// (cascade resolve via `ContextJson`) needs toolheads + build
/// volume + exclusion zones too. Single fetch, full info beats
/// a per-row identity → profile round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub identity: String,
    pub profile: PrinterProfile,
    #[serde(default)]
    pub experimental: bool,
}

/// Picker-side view of every bundled profile, in declaration order.
/// Clones from the library because `PrinterProfile` is serialized
/// across the Tauri IPC boundary by value and the picker shape
/// (`CatalogEntry`) reorders fields.
pub fn bundled_catalog() -> Vec<CatalogEntry> {
    profile_library::printer_catalog()
        .iter()
        .map(|e| CatalogEntry {
            identity: e.identity.clone(),
            profile: hydrate_profile(&e.profile, &e.fragment_slug),
            experimental: e.experimental,
        })
        .collect()
}

/// Resolve a bundled identity to its full `PrinterProfile`. Returns
/// `None` for unknown identities. The Tauri command layer maps that
/// to a `String` error the picker shows as a toast.
pub fn lookup(identity: &str) -> Option<PrinterProfile> {
    profile_library::printer_catalog_lookup(identity)
        .map(|e| hydrate_profile(&e.profile, &e.fragment_slug))
}

/// Fill in the runtime-derived fields on a `PrinterProfile` — the
/// per-printer bed list, available nozzle diameters, the build
/// volume parsed from the machine cascade's `printable_area` /
/// `printable_height`, the `default_bed` pulled from the machine
/// cascade's `default_bed_type` scalar (libslic3r's documented
/// home for the picker default), and each toolhead's `hotend_type`
/// pulled from the per-nozzle profile (the SKU profile carries
/// `nozzle_type` as its own source of truth). model.toml no longer
/// duplicates any of these.
fn hydrate_profile(base: &PrinterProfile, fragment_slug: &str) -> PrinterProfile {
    let mut profile = base.clone();
    profile.supported_build_plates = profile_library::bundled_beds_for_printer(fragment_slug)
        .into_iter()
        .map(str::to_owned)
        .collect();
    profile.available_nozzle_diameters = profile_library::nozzle_skus_for(fragment_slug)
        .into_iter()
        .map(str::to_owned)
        .collect();
    if let Some(bv) = profile_library::build_volume_for_printer(fragment_slug) {
        profile.build_volume = bv;
    }
    if let Some(bed) = profile_library::default_bed_type_for(fragment_slug) {
        profile.default_bed = Some(bed);
    }
    for toolhead in profile.toolheads.iter_mut() {
        if let Some(nozzle_type) =
            profile_library::nozzle_type_for(fragment_slug, &toolhead.default_nozzle_diameter)
        {
            toolhead.hotend_type = nozzle_type;
        }
    }
    // `driver_kind` is authored directly in `model.toml` (parsed into
    // `base.driver_kind`), so it carries through the clone untouched.
    // A printer whose `model.toml` omits it resolves to `None` — we
    // ship no driver for it and the settings modal hides its
    // Connection tab.
    profile
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_contains_a1_mini_and_u1() {
        let entries = bundled_catalog();
        assert!(entries.iter().any(|e| e.identity == "bambu-lab-a1-mini"));
        assert!(entries.iter().any(|e| e.identity == "snapmaker-u1"));
        let a1 = entries
            .iter()
            .find(|e| e.identity == "bambu-lab-a1-mini")
            .unwrap();
        assert_eq!(a1.profile.model, "Bambu Lab A1 mini");
        let u1 = entries
            .iter()
            .find(|e| e.identity == "snapmaker-u1")
            .unwrap();
        assert_eq!(u1.profile.model, "Snapmaker U1");
    }

    #[test]
    fn lookup_resolves_known_identities() {
        let a1 = lookup("bambu-lab-a1-mini").expect("a1 mini present");
        assert_eq!(a1.model, "Bambu Lab A1 mini");
        assert_eq!(a1.toolheads.len(), 1);

        let u1 = lookup("snapmaker-u1").expect("u1 present");
        assert_eq!(u1.model, "Snapmaker U1");
        assert_eq!(u1.toolheads.len(), 4);
    }

    #[test]
    fn lookup_returns_none_for_unknown_identity() {
        assert!(lookup("totally-fake-printer").is_none());
    }

    #[test]
    fn driver_kind_declared_in_model_toml() {
        // driver_kind is authored in each printer's model.toml. Pinning
        // so a future Prusa/Voron/Creality landing without a driver
        // (model.toml omits the field → None) doesn't accidentally
        // inherit Bambu (the old App.tsx hand-rolled fallback).
        use super::super::super::driver::traits::DriverKind;
        let a1 = lookup("bambu-lab-a1-mini").expect("a1 mini present");
        assert_eq!(a1.driver_kind, Some(DriverKind::Bambu));
        let u1 = lookup("snapmaker-u1").expect("u1 present");
        assert_eq!(u1.driver_kind, Some(DriverKind::U1));
    }

    fn machine_scalar(slug: &str, key: &str) -> Option<String> {
        crate::core::profile_library::load_printer_fragment(slug)
            .expect("machine cascade")
            .rules
            .iter()
            .find(|r| r.is_default())
            .and_then(|r| r.set.get(key))
            .cloned()
    }

    #[test]
    fn ender_3_s1_base_is_a_driverless_marlin_import() {
        // The base S1 is the verbatim upstream import: Marlin flavor,
        // no LAN driver (stock firmware has no control plane we speak).
        let s1 = lookup("creality-ender-3-s1").expect("ender-3 s1 present");
        assert_eq!(s1.model, "Creality Ender-3 S1");
        assert_eq!(s1.driver_kind, None);
        assert_eq!(s1.toolheads.len(), 1);
        assert_eq!(s1.ams_max, 0);
        assert_eq!(s1.build_volume.max[2], 270.0);
        assert_eq!(s1.default_bed.as_deref(), Some("Textured PEI Plate"));
        assert_eq!(
            machine_scalar("creality-ender-3-s1", "gcode_flavor").as_deref(),
            Some("marlin"),
        );
    }

    #[test]
    fn ender_3_s1_klipper_variant_derives_and_patches() {
        // The Klipper conversion is DERIVED from the base by
        // scripts/derive_printer_variant.py: own model name (picker +
        // cascade key), klipper flavor, generic Moonraker driver. Its
        // process fragments' `when.printer.model` predicates must
        // follow the renamed model or quality profiles vanish.
        use super::super::super::driver::traits::DriverKind;
        let s1k = lookup("creality-ender-3-s1-klipper").expect("klipper variant present");
        assert_eq!(s1k.model, "Creality Ender-3 S1 (Klipper)");
        assert_eq!(s1k.driver_kind, Some(DriverKind::Moonraker));
        assert_eq!(s1k.default_bed.as_deref(), Some("Textured PEI Plate"));
        assert_eq!(
            machine_scalar("creality-ender-3-s1-klipper", "gcode_flavor").as_deref(),
            Some("klipper"),
        );
        let processes = crate::core::profile_library::list_process_fragments(
            "creality-ender-3-s1-klipper",
            "Creality Ender-3 S1 (Klipper)",
            &["0.4".to_owned()],
        );
        assert!(
            !processes.is_empty(),
            "derived processes must fire under the variant's model name",
        );
    }

    #[test]
    fn creality_profiles_are_experimental() {
        let entries = bundled_catalog();
        for identity in [
            "creality-ender-3-s1",
            "creality-ender-3-s1-klipper",
            "creality-ender-3-v3-ke",
        ] {
            let entry = entries
                .iter()
                .find(|e| e.identity == identity)
                .unwrap_or_else(|| panic!("{identity} in catalog"));
            assert!(entry.experimental, "{identity} must be experimental");
        }
    }

    #[test]
    fn ender_3_v3_ke_imports_verbatim_as_klipper_moonraker() {
        // Natively-Klipper upstream profile — no derived variant, the
        // plain import carries the flavor. Four nozzle SKUs.
        use super::super::super::driver::traits::DriverKind;
        let ke = lookup("creality-ender-3-v3-ke").expect("v3 ke present");
        assert_eq!(ke.model, "Creality Ender-3 V3 KE");
        assert_eq!(ke.driver_kind, Some(DriverKind::Moonraker));
        assert_eq!(ke.toolheads.len(), 1);
        assert_eq!(ke.build_volume.max, [220.0, 220.0, 245.0]);
        assert_eq!(
            ke.available_nozzle_diameters,
            vec!["0.2", "0.4", "0.6", "0.8"],
        );
        assert_eq!(
            machine_scalar("creality-ender-3-v3-ke", "gcode_flavor").as_deref(),
            Some("klipper"),
        );
        let processes = crate::core::profile_library::list_process_fragments(
            "creality-ender-3-v3-ke",
            "Creality Ender-3 V3 KE",
            &["0.4".to_owned()],
        );
        assert!(!processes.is_empty());
    }

    #[test]
    fn catalog_entry_carries_full_profile_for_panel_resolve() {
        let entries = bundled_catalog();
        let a1 = entries
            .iter()
            .find(|e| e.identity == "bambu-lab-a1-mini")
            .unwrap();
        assert!(a1
            .profile
            .supported_build_plates
            .contains(&"Textured PEI Plate".into()));
        assert_eq!(a1.profile.toolheads.len(), 1);
        // A1 mini ships per-nozzle fragments for 0.2 / 0.4 / 0.6 / 0.8.
        assert_eq!(
            a1.profile.available_nozzle_diameters,
            vec!["0.2", "0.4", "0.6", "0.8"],
        );
        // Build volume is hydrated from the machine cascade's
        // `printable_area` / `printable_height`; A1 mini is 180³.
        assert_eq!(a1.profile.build_volume.max, [180.0, 180.0, 180.0]);

        let u1 = entries
            .iter()
            .find(|e| e.identity == "snapmaker-u1")
            .unwrap();
        assert_eq!(u1.profile.toolheads.len(), 4);
        // U1 cascade: printable_area "0.5x1,270.5x1,270.5x271,0.5x271",
        // printable_height "270.05".
        assert_eq!(u1.profile.build_volume.max, [270.5, 271.0, 270.05]);
        assert_eq!(u1.profile.build_volume.min, [0.5, 1.0, 0.0]);
    }
}
