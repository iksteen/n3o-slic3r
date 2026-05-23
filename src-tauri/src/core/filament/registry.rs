//! Bundled filament profile registry.
//!
//! Mirrors `core::printer::registry` for filament identities. The
//! cascade context (PR-1-7) wants `Vec<FilamentProfile>`, the
//! per-plate material bindings (PR-5-6) point at identities; this
//! registry is the bridge.
//!
//! Phase 5/6 ship `Generic PLA` as the only bundled entry — every
//! plate's auto-bind defaults to it (see
//! `Project::ensure_default_material_binding_on_active`). Real
//! filament catalog work lands post-MVP alongside the filament-sync
//! UX (Phase 7c).
//!
//! Surface:
//! - [`lookup(identity)`] — `Option<FilamentProfile>` for bundled
//!   identities.
//! - [`bundled_catalog()`] — every bundled identity + its profile.

use super::profile::FilamentProfile;

#[derive(Debug, Clone, Copy)]
struct BundledFilament {
    identity: &'static str,
    toml: &'static str,
}

const GENERIC_PLA_TOML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../profiles/filaments/generic-pla.toml"
));

const BUNDLED: &[BundledFilament] = &[BundledFilament {
    identity: "Generic PLA",
    toml: GENERIC_PLA_TOML,
}];

/// Resolve a bundled identity to its full `FilamentProfile`. Returns
/// `None` for unknown identities — callers fall back to a synthesized
/// stand-in (slice context still needs *some* filament; the cascade
/// uses `base_type` to pick PLA-flavor rules).
pub fn lookup(identity: &str) -> Option<FilamentProfile> {
    BUNDLED
        .iter()
        .find(|b| b.identity == identity)
        .map(|b| {
            toml::from_str::<FilamentProfile>(b.toml).unwrap_or_else(|e| {
                panic!("bundled filament `{}`: {e}", b.identity)
            })
        })
}

/// Full bundled catalog in declaration order. Useful for UI pickers
/// once a real filament-picker lands; currently the frontend stubs a
/// 4-entry list in `material/filamentCatalog.ts` for the binding
/// panel.
pub fn bundled_catalog() -> Vec<FilamentProfile> {
    BUNDLED
        .iter()
        .map(|b| {
            toml::from_str::<FilamentProfile>(b.toml).unwrap_or_else(|e| {
                panic!("bundled filament `{}`: {e}", b.identity)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_resolves_generic_pla() {
        let f = lookup("Generic PLA").expect("Generic PLA present");
        assert_eq!(f.identity, "Generic PLA");
        assert_eq!(f.base_type, "PLA");
    }

    #[test]
    fn lookup_returns_none_for_unknown() {
        assert!(lookup("Pretend-PLA").is_none());
    }

    #[test]
    fn bundled_catalog_contains_generic_pla() {
        let catalog = bundled_catalog();
        assert!(catalog.iter().any(|f| f.identity == "Generic PLA"));
    }
}
