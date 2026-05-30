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

/// Identity of the first printer in the bundled vendor catalog
/// (alphabetical by slug). Only test setups call this — production
/// resolves the active plate's printer from the user library on
/// disk via `instance_storage` instead. Returns `None` only if the
/// vendor catalog itself is empty.
pub fn default_printer_identity() -> Option<&'static str> {
    DEFAULT_IDENTITY
        .get_or_init(|| bundled_catalog().into_iter().next().map(|e| e.identity))
        .as_deref()
}

static DEFAULT_IDENTITY: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

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
    fn default_picks_first_printer() {
        // Returns the first identity in catalog order (which the
        // library iterates in sorted-on-disk vendor → slug order).
        // The specific identity isn't the property under test —
        // adding a new printer that sorts earlier shouldn't break
        // this — only that *some* catalog entry comes back, and it
        // matches whichever one bundled_catalog yields first.
        let first = bundled_catalog().into_iter().next().map(|e| e.identity);
        assert_eq!(default_printer_identity(), first.as_deref());
        assert!(default_printer_identity().is_some());
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
