//! Printer profile registry (PR-5-4).
//!
//! Holds the bundled printer profiles so the frontend has something
//! concrete to pick between in the settings panel's printer picker.
//! Each TOML in `profiles/printers/` is embedded via `include_str!`
//! at compile time — same pattern the cascade registry uses for the
//! bundled A1 mini cascade. Packaged builds work without runtime
//! resource lookup.
//!
//! Surface:
//! - [`bundled_catalog()`] — returns the full set of registered
//!   profiles. Stable order: A1 mini first (the MVP default), then
//!   Snapmaker U1, then anything else added in declaration order.
//! - [`lookup(identity)`] — find a profile by its identity slug
//!   (the profile filename stem, e.g. `"bambu-a1-mini"`).
//! - [`CatalogEntry`] — the small summary the picker UI consumes
//!   without having to round-trip the full profile.
//!
//! The registry is content-only — every call re-parses the embedded
//! TOML. Parse cost is single-digit ms even on debug builds, far
//! less than any cascade resolve, and we hit it once per app launch
//! plus once per printer change. Caching adds complexity (Mutex on
//! a lazy_static) for no measurable benefit.

use serde::{Deserialize, Serialize};

use super::profile::PrinterProfile;

/// One bundled printer profile: identity slug + the embedded TOML
/// body. Identity is the stable string the frontend passes to
/// `scene_set_plate_printer_by_identity` to swap printers on a plate.
#[derive(Debug, Clone, Copy)]
struct BundledProfile {
    identity: &'static str,
    toml: &'static str,
}

const BAMBU_A1_MINI_TOML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../profiles/printers/bambu-a1-mini.toml"
));
const SNAPMAKER_U1_TOML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../profiles/printers/snapmaker-u1.toml"
));

/// Declaration-ordered list of bundled printers. Insertion order is
/// also the picker's display order; the MVP A1 mini sits at the top
/// because that's what the bootstrap loads.
const BUNDLED: &[BundledProfile] = &[
    BundledProfile {
        identity: "bambu-a1-mini",
        toml: BAMBU_A1_MINI_TOML,
    },
    BundledProfile {
        identity: "snapmaker-u1",
        toml: SNAPMAKER_U1_TOML,
    },
];

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

impl CatalogEntry {
    fn from(identity: &str, profile: PrinterProfile) -> Self {
        Self {
            identity: identity.to_owned(),
            profile,
        }
    }
}

/// Picker-side view of every bundled profile, in display order.
/// Panics on TOML parse failure — the embedded strings are
/// compile-time constants, so a parse failure means the bundled
/// profile is malformed and the binary shouldn't have shipped.
pub fn bundled_catalog() -> Vec<CatalogEntry> {
    BUNDLED
        .iter()
        .map(|b| {
            let profile = parse(b.toml)
                .unwrap_or_else(|e| panic!("bundled printer `{}`: {e}", b.identity));
            CatalogEntry::from(b.identity, profile)
        })
        .collect()
}

/// Resolve a bundled identity to its full `PrinterProfile`. Returns
/// `None` for unknown identities. The Tauri command layer maps that
/// to a `String` error the picker shows as a toast.
pub fn lookup(identity: &str) -> Option<PrinterProfile> {
    BUNDLED
        .iter()
        .find(|b| b.identity == identity)
        .map(|b| {
            parse(b.toml)
                .unwrap_or_else(|e| panic!("bundled printer `{identity}`: {e}"))
        })
}

fn parse(toml: &str) -> Result<PrinterProfile, toml::de::Error> {
    toml::from_str::<PrinterProfile>(toml)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_contains_a1_mini_and_u1() {
        let entries = bundled_catalog();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].identity, "bambu-a1-mini");
        assert_eq!(entries[0].profile.model, "Bambu A1 mini");
        assert_eq!(entries[1].identity, "snapmaker-u1");
        assert_eq!(entries[1].profile.model, "Snapmaker U1");
    }

    #[test]
    fn lookup_resolves_known_identities() {
        let a1 = lookup("bambu-a1-mini").expect("a1 mini present");
        assert_eq!(a1.model, "Bambu A1 mini");
        assert_eq!(a1.slot_count, 4);
        // One AMS-fed toolhead.
        assert_eq!(a1.toolheads.len(), 1);

        let u1 = lookup("snapmaker-u1").expect("u1 present");
        assert_eq!(u1.model, "Snapmaker U1");
        assert_eq!(u1.slot_count, 4);
        // Four toolchanger toolheads.
        assert_eq!(u1.toolheads.len(), 4);
    }

    #[test]
    fn lookup_returns_none_for_unknown_identity() {
        assert!(lookup("totally-fake-printer").is_none());
    }

    #[test]
    fn catalog_entry_carries_full_profile_for_panel_resolve() {
        // The settings panel host derives the active printer's
        // PrinterProfileJson from the catalog entry (cascade resolve
        // wants toolheads + build volume + exclusion zones, not just
        // the picker summary). Pin the shape so a future shrink of
        // CatalogEntry that drops fields surfaces here.
        let entries = bundled_catalog();
        let a1 = entries.iter().find(|e| e.identity == "bambu-a1-mini").unwrap();
        assert!(a1.profile.supported_build_plates.contains(&"Textured PEI".into()));
        assert_eq!(a1.profile.toolheads.len(), 1);
        assert!(a1.profile.build_volume.max[0] > 0.0);

        let u1 = entries.iter().find(|e| e.identity == "snapmaker-u1").unwrap();
        assert_eq!(u1.profile.toolheads.len(), 4);
        assert!(!u1.profile.exclusion_zones.is_empty());
    }
}
