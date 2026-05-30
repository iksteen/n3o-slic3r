//! Build plate descriptor.
//!
//! Phase 1 ships just the identity + libslic3r-side bed-type mapping
//! the cascade adapter needs to set `curr_bed_type`. Phase
//! 2's scene state extends this with mesh + adhesion + visual
//! properties.

use serde::{Deserialize, Serialize};

/// A build plate the active printer supports.
///
/// `identity` matches libslic3r's `curr_bed_type` enum vocabulary
/// verbatim (e.g. `"Textured PEI Plate"`, `"Supertack Plate"`). The
/// `libslic3r_curr_bed_type` field is the same string and exists for
/// historical reasons — earlier the picker used short identities
/// ("Textured PEI") and this descriptor translated them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildPlate {
    pub identity: String,
    pub libslic3r_curr_bed_type: String,
}

/// Resolve a build-plate identity to its descriptor. The library's
/// bed fragments are vendor-scoped (per-printer); for the picker
/// fallback path we accept any identity that any bundled printer
/// supports — the cascade composer is the only place where the
/// printer-scoped lookup actually matters.
///
/// Returns `None` for plates not present in any bundled bed
/// fragment — callers synthesize a fallback descriptor at the call
/// site (cascade still needs *some* `libslic3r_curr_bed_type` to
/// write into the slice config).
pub fn lookup(identity: &str) -> Option<BuildPlate> {
    // The bed identity is its own libslic3r `curr_bed_type` enum
    // value verbatim — see the vendor bed.toml fragments. The
    // descriptor exposes the two as separate fields purely for
    // historical reasons; the values match.
    let lib = crate::core::profile_library::printer_catalog();
    let known = lib.iter().any(|entry| {
        crate::core::profile_library::bundled_beds_for_printer(&entry.fragment_slug)
            .contains(&identity)
    });
    if known {
        Some(BuildPlate {
            identity: identity.to_owned(),
            libslic3r_curr_bed_type: identity.to_owned(),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_toml() {
        let p = BuildPlate {
            identity: "Textured PEI Plate".into(),
            libslic3r_curr_bed_type: "Textured PEI Plate".into(),
        };
        let text = toml::to_string(&p).expect("serialize");
        let parsed: BuildPlate = toml::from_str(&text).expect("deserialize");
        assert_eq!(parsed.identity, "Textured PEI Plate");
        assert_eq!(parsed.libslic3r_curr_bed_type, "Textured PEI Plate");
    }

    #[test]
    fn lookup_resolves_textured_pei() {
        let p = lookup("Textured PEI Plate").expect("Textured PEI present");
        assert_eq!(p.identity, "Textured PEI Plate");
        assert_eq!(p.libslic3r_curr_bed_type, "Textured PEI Plate");
    }

    #[test]
    fn lookup_returns_none_for_unbundled_plate() {
        assert!(lookup("Magnetic").is_none());
    }
}
