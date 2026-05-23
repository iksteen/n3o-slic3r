//! Build plate descriptor.
//!
//! Phase 1 ships just the identity + libslic3r-side bed-type mapping
//! the cascade adapter (PR-1-6) needs to set `curr_bed_type`. Phase
//! 2's scene state extends this with mesh + adhesion + visual
//! properties.

use serde::{Deserialize, Serialize};

/// A build plate the active printer supports.
///
/// `identity` is the cascade-side name (`"Textured PEI"`,
/// `"SuperTack"`) — appears in cascade predicates as `plate.type`
/// values. `libslic3r_curr_bed_type` is the corresponding string
/// libslic3r's `curr_bed_type` enum accepts (`"Textured PEI Plate"`,
/// `"Supertack Plate"`); the adapter writes this verbatim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildPlate {
    pub identity: String,
    pub libslic3r_curr_bed_type: String,
    pub surface_kind: SurfaceKind,
}

/// Categorical surface kind. Drives adhesion guidance + (Phase 2)
/// renderer texture choice. Not currently in cascade predicates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SurfaceKind {
    PEI,
    SuperTack,
    Engineering,
    Cool,
    Other,
}

// ---- Bundled-plate registry ---------------------------------------

#[derive(Debug, Clone, Copy)]
struct BundledPlate {
    identity: &'static str,
    toml: &'static str,
}

const TEXTURED_PEI_TOML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../profiles/plates/textured-pei.toml"
));

const BUNDLED: &[BundledPlate] = &[BundledPlate {
    identity: "Textured PEI",
    toml: TEXTURED_PEI_TOML,
}];

/// Resolve a build-plate identity to its full descriptor. Returns
/// `None` for plates not present in the bundled set — callers
/// synthesize a fallback (cascade still needs *some*
/// `libslic3r_curr_bed_type`; a best-effort `format!("{identity}
/// Plate")` suffices for plates whose TOML hasn't been authored
/// yet).
///
/// Authoring more bundled plates is post-MVP profile work; the
/// printer registry's `supported_build_plates` lists identities the
/// picker shows even when no plate TOML exists for them.
pub fn lookup(identity: &str) -> Option<BuildPlate> {
    BUNDLED
        .iter()
        .find(|b| b.identity == identity)
        .map(|b| {
            toml::from_str::<BuildPlate>(b.toml).unwrap_or_else(|e| {
                panic!("bundled plate `{}`: {e}", b.identity)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_toml() {
        let p = BuildPlate {
            identity: "Textured PEI".into(),
            libslic3r_curr_bed_type: "Textured PEI Plate".into(),
            surface_kind: SurfaceKind::PEI,
        };
        let text = toml::to_string(&p).expect("serialize");
        let parsed: BuildPlate = toml::from_str(&text).expect("deserialize");
        assert_eq!(parsed.identity, "Textured PEI");
        assert_eq!(parsed.libslic3r_curr_bed_type, "Textured PEI Plate");
        assert_eq!(parsed.surface_kind, SurfaceKind::PEI);
    }

    #[test]
    fn lookup_resolves_bundled_textured_pei() {
        let p = lookup("Textured PEI").expect("Textured PEI present");
        assert_eq!(p.identity, "Textured PEI");
        assert_eq!(p.libslic3r_curr_bed_type, "Textured PEI Plate");
        assert_eq!(p.surface_kind, SurfaceKind::PEI);
    }

    #[test]
    fn lookup_returns_none_for_unbundled_plate() {
        // `Magnetic` is in snapmaker-u1's supported_build_plates list
        // but has no bundled TOML yet — callers synthesize a fallback.
        assert!(lookup("Magnetic").is_none());
    }
}
