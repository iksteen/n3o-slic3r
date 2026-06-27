//! Bundled filament profile registry.
//!
//! Thin facade over [`crate::core::profile_library::list_filament_fragments`].
//! Each vendor filament fragment under `profiles/<v>/filament/`
//! carries the `filament_settings_id` and `filament_type` fields the
//! cascade context needs to construct a [`FilamentProfile`]; we derive
//! the picker-facing struct from those instead of keeping a parallel
//! n3o-shape filament catalog on disk.
//!
//! Surface:
//! - [`lookup(identity)`] — `Option<FilamentProfile>` for bundled
//!   fragments.
//! - [`bundled_catalog()`] — every bundled identity + its profile.

use super::profile::FilamentProfile;
use crate::core::profile_library;

fn build_profile(summary: profile_library::FilamentFragmentSummary) -> FilamentProfile {
    FilamentProfile {
        identity: summary.identity,
        base_type: summary.base_type,
        vendor: None,
        color: None,
    }
}

/// Resolve a bundled identity (fragment slug) to a `FilamentProfile`.
/// Returns `None` for unknown identities — callers fall back to a
/// synthesized stand-in (slice context still needs *some* filament;
/// the cascade uses `base_type` to pick PLA-flavor rules).
pub fn lookup(identity: &str) -> Option<FilamentProfile> {
    profile_library::filament_fragment_summary(identity).map(build_profile)
}

/// Full bundled catalog in declaration order.
pub fn bundled_catalog() -> Vec<FilamentProfile> {
    profile_library::list_filament_fragments()
        .into_iter()
        .map(build_profile)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_resolves_generic_pla() {
        let f = lookup("generic-pla").expect("generic-pla present");
        assert_eq!(f.identity, "generic-pla");
        assert_eq!(f.base_type, "PLA");
    }

    #[test]
    fn lookup_returns_none_for_unknown() {
        assert!(lookup("Pretend-PLA").is_none());
    }

    #[test]
    fn bundled_catalog_contains_generic_pla() {
        let catalog = bundled_catalog();
        assert!(catalog.iter().any(|f| f.identity == "generic-pla"));
    }
}
