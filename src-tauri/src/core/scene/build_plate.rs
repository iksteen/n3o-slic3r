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
}
