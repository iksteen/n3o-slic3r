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
/// per-printer bed list and the build volume parsed from the
/// machine cascade's `printable_area` / `printable_height`. Kept
/// as one place so the catalog walk + single-id `lookup` can't
/// drift. `default_bed` is loaded by serde directly from
/// `model.toml` and needs no hydration.
fn hydrate_profile(base: &PrinterProfile, fragment_slug: &str) -> PrinterProfile {
    let mut profile = base.clone();
    profile.supported_build_plates =
        profile_library::bundled_beds_for_printer(fragment_slug)
            .into_iter()
            .map(str::to_owned)
            .collect();
    if let Some(bv) = profile_library::build_volume_for_printer(fragment_slug) {
        profile.build_volume = bv;
    }
    profile
}

/// Identity of the bundled-default printer. Used to auto-bind a
/// fresh plate to the bundled instance for that printer so first-
/// launch slicing works without forcing the user through the
/// picker.
///
/// Returns `None` only if the bundled catalog is empty.
pub fn default_printer_identity() -> Option<&'static str> {
    DEFAULT_IDENTITY.get_or_init(|| {
        bundled_catalog().into_iter().next().map(|e| e.identity)
    }).as_deref()
}

static DEFAULT_IDENTITY: std::sync::OnceLock<Option<String>> =
    std::sync::OnceLock::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_contains_a1_mini_and_u1() {
        let entries = bundled_catalog();
        assert!(entries.iter().any(|e| e.identity == "bambu-lab-a1-mini"));
        assert!(entries.iter().any(|e| e.identity == "snapmaker-u1"));
        let a1 = entries.iter().find(|e| e.identity == "bambu-lab-a1-mini").unwrap();
        assert_eq!(a1.profile.model, "Bambu A1 mini");
        let u1 = entries.iter().find(|e| e.identity == "snapmaker-u1").unwrap();
        assert_eq!(u1.profile.model, "Snapmaker U1");
    }

    #[test]
    fn lookup_resolves_known_identities() {
        let a1 = lookup("bambu-lab-a1-mini").expect("a1 mini present");
        assert_eq!(a1.model, "Bambu A1 mini");
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
        assert_eq!(default_printer_identity(), Some("bambu-lab-a1-mini"));
    }

    #[test]
    fn catalog_entry_carries_full_profile_for_panel_resolve() {
        let entries = bundled_catalog();
        let a1 = entries.iter().find(|e| e.identity == "bambu-lab-a1-mini").unwrap();
        assert!(a1.profile.supported_build_plates.contains(&"Textured PEI Plate".into()));
        assert_eq!(a1.profile.toolheads.len(), 1);
        // Build volume is hydrated from the machine cascade's
        // `printable_area` / `printable_height`; A1 mini is 180³.
        assert_eq!(a1.profile.build_volume.max, [180.0, 180.0, 180.0]);

        let u1 = entries.iter().find(|e| e.identity == "snapmaker-u1").unwrap();
        assert_eq!(u1.profile.toolheads.len(), 4);
        // U1 cascade: printable_area "0.5x1,270.5x1,270.5x271,0.5x271",
        // printable_height "270.05".
        assert_eq!(u1.profile.build_volume.max, [270.5, 271.0, 270.05]);
        assert_eq!(u1.profile.build_volume.min, [0.5, 1.0, 0.0]);
    }
}
