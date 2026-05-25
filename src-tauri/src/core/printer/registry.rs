//! Printer profile registry.
//!
//! Thin facade over [`crate::core::profile_library::printer_catalog`].
//! The library walks the vendor profile tree at startup and parses one
//! `printer.toml` per printer directory; this module re-exposes that
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
use crate::core::project::binding::PrinterBinding;

/// Picker-facing entry for one printer in the catalog. Carries the
/// identity slug + the full `PrinterProfile`. The picker chip + menu
/// only read `identity`, `profile.model`, `profile.slot_count`, and
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
        .map(|e| {
            let mut profile = e.profile.clone();
            profile.supported_build_plates =
                profile_library::bundled_beds_for_printer(&e.fragment_slug)
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
            CatalogEntry {
                identity: e.identity.clone(),
                profile,
            }
        })
        .collect()
}

/// Resolve a bundled identity to its full `PrinterProfile`. Returns
/// `None` for unknown identities. The Tauri command layer maps that
/// to a `String` error the picker shows as a toast.
pub fn lookup(identity: &str) -> Option<PrinterProfile> {
    profile_library::printer_catalog_lookup(identity).map(|e| {
        let mut profile = e.profile.clone();
        profile.supported_build_plates =
            profile_library::bundled_beds_for_printer(&e.fragment_slug)
                .into_iter()
                .map(str::to_owned)
                .collect();
        profile
    })
}

/// Best-guess default printer binding for fresh projects + newly-
/// added plates whose caller didn't pick a printer. Mirrors the
/// auto-bind-materials pattern: the user gets a sensible starting
/// state and changes it explicitly via the picker if it doesn't
/// match their actual printer.
///
/// Picks the first bundled printer + its first supported build
/// plate. Returns `None` only if the bundled catalog is empty or
/// the first printer ships with no build plates.
pub fn default_binding() -> Option<PrinterBinding> {
    let entry = bundled_catalog().into_iter().next()?;
    let build_plate_identity = entry.profile.supported_build_plates.into_iter().next()?;
    Some(PrinterBinding {
        printer_identity: entry.identity,
        build_plate_identity,
    })
}

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
        assert_eq!(a1.slot_count, 4);
        assert_eq!(a1.toolheads.len(), 1);

        let u1 = lookup("snapmaker-u1").expect("u1 present");
        assert_eq!(u1.model, "Snapmaker U1");
        assert_eq!(u1.slot_count, 4);
        assert_eq!(u1.toolheads.len(), 4);
    }

    #[test]
    fn lookup_returns_none_for_unknown_identity() {
        assert!(lookup("totally-fake-printer").is_none());
    }

    #[test]
    fn catalog_entry_carries_full_profile_for_panel_resolve() {
        let entries = bundled_catalog();
        let a1 = entries.iter().find(|e| e.identity == "bambu-lab-a1-mini").unwrap();
        assert!(a1.profile.supported_build_plates.contains(&"Textured PEI Plate".into()));
        assert_eq!(a1.profile.toolheads.len(), 1);
        assert!(a1.profile.build_volume.max[0] > 0.0);

        let u1 = entries.iter().find(|e| e.identity == "snapmaker-u1").unwrap();
        assert_eq!(u1.profile.toolheads.len(), 4);
        assert!(!u1.profile.exclusion_zones.is_empty());
    }
}
