//! Hierarchical vendor profile library (PR-S-4 rework).
//!
//! Loads the bundled vendor profile fragments from the hierarchical
//! layout:
//!
//! ```text
//! profiles/vendor/<vendor>/
//! ├── printer/
//! │   ├── <slug>.toml              ← machine globals only
//! │   └── <slug>/
//! │       ├── nozzles/<sku>.toml   ← per-extruder scalars
//! │       └── beds/<slug>.toml     ← thin metadata (identity, curr_bed_type)
//! ├── filament/<slug>.toml
//! └── process/<slug>.toml
//! ```
//!
//! Beds live under the printer (sibling to nozzles) — the supported
//! plate list is printer-specific (A1 mini's Supertack vs the U1's
//! textured PEI etc.) and the bed identity vocabulary is shared with
//! libslic3r's `curr_bed_type` enum so the picker, cascade, and FFI
//! all speak the same strings.
//!
//! Each fragment is `include_str!`-bundled at compile time. The
//! composer (`composer::compose_cascade`) layers them into a slice-
//! time Cascade with per-extruder vector assembly for nozzle fragments.
//!
//! Per-bucket OptionDef classification (PR-S-1) is a separate
//! concern — it drives UI editing routes. File layout reflects
//! physical sub-component ownership (printer / nozzle / bed); bucket
//! reflects semantic category (printer / filament / process). The two
//! are orthogonal.

use std::path::Path;

use crate::core::cascade::loader::{parse_cascade_str, CascadeLoadError};
use crate::core::cascade::types::Cascade;

pub mod composer;
pub use composer::{compose_cascade, ComposeError};

/// One bundled fragment along with the path the cascade loader uses
/// as its SourceLocation (for trace readability).
#[derive(Debug, Clone, Copy)]
struct BundledFragment {
    slug: &'static str,
    toml: &'static str,
    source_path: &'static str,
}

/// One bundled nozzle fragment scoped to a printer.
#[derive(Debug, Clone, Copy)]
struct BundledNozzle {
    printer_slug: &'static str,
    sku: &'static str,
    toml: &'static str,
    source_path: &'static str,
}

/// One bundled bed fragment scoped to a printer. `identity` is the
/// libslic3r `curr_bed_type` enum value the bed.toml carries
/// (e.g. `"Supertack Plate"`); the cascade composer looks up the
/// fragment by `(printer_slug, identity)`.
#[derive(Debug, Clone, Copy)]
struct BundledBed {
    printer_slug: &'static str,
    identity: &'static str,
    toml: &'static str,
    source_path: &'static str,
}

// ---- Printers -------------------------------------------------------

const PRINTERS: &[BundledFragment] = &[
    BundledFragment {
        slug: "bambu-lab-a1-mini",
        toml: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/bbl/printer/bambu-lab-a1-mini.toml"
        )),
        source_path: "profiles/vendor/bbl/printer/bambu-lab-a1-mini.toml",
    },
    BundledFragment {
        slug: "snapmaker-u1",
        toml: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/snapmaker/printer/snapmaker-u1.toml"
        )),
        source_path: "profiles/vendor/snapmaker/printer/snapmaker-u1.toml",
    },
];

// ---- Nozzles (per-printer, per-SKU) --------------------------------

const NOZZLES: &[BundledNozzle] = &[
    // Bambu A1 mini nozzle range.
    BundledNozzle {
        printer_slug: "bambu-lab-a1-mini", sku: "0.2",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/bbl/printer/bambu-lab-a1-mini/nozzles/0.2.toml")),
        source_path: "profiles/vendor/bbl/printer/bambu-lab-a1-mini/nozzles/0.2.toml",
    },
    BundledNozzle {
        printer_slug: "bambu-lab-a1-mini", sku: "0.4",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/bbl/printer/bambu-lab-a1-mini/nozzles/0.4.toml")),
        source_path: "profiles/vendor/bbl/printer/bambu-lab-a1-mini/nozzles/0.4.toml",
    },
    BundledNozzle {
        printer_slug: "bambu-lab-a1-mini", sku: "0.6",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/bbl/printer/bambu-lab-a1-mini/nozzles/0.6.toml")),
        source_path: "profiles/vendor/bbl/printer/bambu-lab-a1-mini/nozzles/0.6.toml",
    },
    BundledNozzle {
        printer_slug: "bambu-lab-a1-mini", sku: "0.8",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/bbl/printer/bambu-lab-a1-mini/nozzles/0.8.toml")),
        source_path: "profiles/vendor/bbl/printer/bambu-lab-a1-mini/nozzles/0.8.toml",
    },
    // Snapmaker U1 nozzle range.
    BundledNozzle {
        printer_slug: "snapmaker-u1", sku: "0.4",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/snapmaker/printer/snapmaker-u1/nozzles/0.4.toml")),
        source_path: "profiles/vendor/snapmaker/printer/snapmaker-u1/nozzles/0.4.toml",
    },
    BundledNozzle {
        printer_slug: "snapmaker-u1", sku: "0.6",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/snapmaker/printer/snapmaker-u1/nozzles/0.6.toml")),
        source_path: "profiles/vendor/snapmaker/printer/snapmaker-u1/nozzles/0.6.toml",
    },
];

// ---- Beds (per-printer) ---------------------------------------------

const BEDS: &[BundledBed] = &[
    // Bambu A1 mini — full plate range. Identities match libslic3r's
    // `s_keys_map_BedType` enum vocabulary verbatim.
    BundledBed {
        printer_slug: "bambu-lab-a1-mini", identity: "Cool Plate",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/bbl/printer/bambu-lab-a1-mini/beds/cool-plate.toml")),
        source_path: "profiles/vendor/bbl/printer/bambu-lab-a1-mini/beds/cool-plate.toml",
    },
    BundledBed {
        printer_slug: "bambu-lab-a1-mini", identity: "Textured PEI Plate",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/bbl/printer/bambu-lab-a1-mini/beds/textured-pei-plate.toml")),
        source_path: "profiles/vendor/bbl/printer/bambu-lab-a1-mini/beds/textured-pei-plate.toml",
    },
    BundledBed {
        printer_slug: "bambu-lab-a1-mini", identity: "High Temp Plate",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/bbl/printer/bambu-lab-a1-mini/beds/high-temp-plate.toml")),
        source_path: "profiles/vendor/bbl/printer/bambu-lab-a1-mini/beds/high-temp-plate.toml",
    },
    BundledBed {
        printer_slug: "bambu-lab-a1-mini", identity: "Engineering Plate",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/bbl/printer/bambu-lab-a1-mini/beds/engineering-plate.toml")),
        source_path: "profiles/vendor/bbl/printer/bambu-lab-a1-mini/beds/engineering-plate.toml",
    },
    BundledBed {
        printer_slug: "bambu-lab-a1-mini", identity: "Supertack Plate",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/bbl/printer/bambu-lab-a1-mini/beds/supertack-plate.toml")),
        source_path: "profiles/vendor/bbl/printer/bambu-lab-a1-mini/beds/supertack-plate.toml",
    },
    // Snapmaker U1 — single plate.
    BundledBed {
        printer_slug: "snapmaker-u1", identity: "Textured PEI Plate",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/snapmaker/printer/snapmaker-u1/beds/textured-pei-plate.toml")),
        source_path: "profiles/vendor/snapmaker/printer/snapmaker-u1/beds/textured-pei-plate.toml",
    },
];

// ---- Filaments + Processes -----------------------------------------

const FILAMENTS: &[BundledFragment] = &[
    BundledFragment {
        slug: "generic-pla",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/orca/filament/generic-pla.toml")),
        source_path: "profiles/vendor/orca/filament/generic-pla.toml",
    },
    BundledFragment {
        slug: "bambu-pla-basic-bbl-a1m",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/bbl/filament/bambu-pla-basic-bbl-a1m.toml")),
        source_path: "profiles/vendor/bbl/filament/bambu-pla-basic-bbl-a1m.toml",
    },
    BundledFragment {
        slug: "snapmaker-pla-u1",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/snapmaker/filament/snapmaker-pla-u1.toml")),
        source_path: "profiles/vendor/snapmaker/filament/snapmaker-pla-u1.toml",
    },
];

const PROCESSES: &[BundledFragment] = &[
    BundledFragment {
        slug: "0.20mm-standard-bbl-a1m",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/bbl/process/0.20mm-standard-bbl-a1m.toml")),
        source_path: "profiles/vendor/bbl/process/0.20mm-standard-bbl-a1m.toml",
    },
    BundledFragment {
        slug: "0.20-standard-snapmaker-u1-0.4-nozzle",
        toml: include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../profiles/vendor/snapmaker/process/0.20-standard-snapmaker-u1-0.4-nozzle.toml")),
        source_path: "profiles/vendor/snapmaker/process/0.20-standard-snapmaker-u1-0.4-nozzle.toml",
    },
];

// ---- Public loaders -------------------------------------------------

fn parse_fragment(slug: &str, toml: &str, source_path: &str) -> Cascade {
    let path = Path::new(source_path);
    let rules = parse_cascade_str(toml, path).unwrap_or_else(|e: CascadeLoadError| {
        panic!("bundled fragment `{slug}` ({source_path}) failed to parse: {e}")
    });
    Cascade { rules }
}

/// Load the printer.toml for `slug`. Returns `None` if unknown.
pub fn load_printer_fragment(slug: &str) -> Option<Cascade> {
    PRINTERS
        .iter()
        .find(|p| p.slug == slug)
        .map(|p| parse_fragment(p.slug, p.toml, p.source_path))
}

/// Load nozzles/<sku>.toml for the named printer + SKU. Returns
/// `None` if either is unknown.
pub fn load_nozzle_fragment(printer_slug: &str, sku: &str) -> Option<Cascade> {
    NOZZLES
        .iter()
        .find(|n| n.printer_slug == printer_slug && n.sku == sku)
        .map(|n| parse_fragment(n.sku, n.toml, n.source_path))
}

/// Load a printer's bed fragment by libslic3r `curr_bed_type`
/// identity (e.g. `("bambu-lab-a1-mini", "Supertack Plate")`).
/// Returns `None` when either the printer or the identity is unknown
/// for that printer.
pub fn load_bed_fragment(printer_slug: &str, identity: &str) -> Option<Cascade> {
    BEDS.iter()
        .find(|b| b.printer_slug == printer_slug && b.identity == identity)
        .map(|b| parse_fragment(b.identity, b.toml, b.source_path))
}

/// Every bed identity bundled for the named printer, in declaration
/// order. The picker enumerates this; falls back to the printer
/// profile's `supported_build_plates` when empty (e.g. a newly-added
/// printer without bed fragments).
pub fn bundled_beds_for_printer(printer_slug: &str) -> Vec<&'static str> {
    BEDS.iter()
        .filter(|b| b.printer_slug == printer_slug)
        .map(|b| b.identity)
        .collect()
}

/// Load filament/<slug>.toml.
pub fn load_filament_fragment(slug: &str) -> Option<Cascade> {
    FILAMENTS
        .iter()
        .find(|f| f.slug == slug)
        .map(|f| parse_fragment(f.slug, f.toml, f.source_path))
}

/// One bundled vendor filament's identity + display label, surfaced
/// to the frontend slot-binding panel. `identity` is the slug
/// (matches the wire form stored in `SlotBinding.filament_identity`);
/// `display_name` is the `filament_settings_id` field a human will
/// recognize ("Bambu PLA Basic @BBL A1M"); `base_type` drives the
/// swatch color in the picker.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FilamentFragmentSummary {
    pub identity: String,
    pub display_name: String,
    pub base_type: String,
}

/// Enumerate every bundled vendor filament fragment. Parses
/// `filament_settings_id` + `filament_type` out of each TOML — both
/// are stamped by the vendor converter and stable across regens.
/// Insertion order is preserved (matches `FILAMENTS` declaration).
pub fn list_filament_fragments() -> Vec<FilamentFragmentSummary> {
    FILAMENTS
        .iter()
        .map(|f| {
            let value: toml::Value = toml::from_str(f.toml).unwrap_or_else(|e| {
                panic!("bundled filament `{}` TOML parse: {e}", f.slug)
            });
            let table = value.as_table().expect("filament fragment is a table");
            let display_name = table
                .get("filament_settings_id")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| f.slug.to_owned());
            let base_type = table
                .get("filament_type")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| "PLA".to_owned());
            FilamentFragmentSummary {
                identity: f.slug.to_owned(),
                display_name,
                base_type,
            }
        })
        .collect()
}

/// Load process/<slug>.toml.
pub fn load_process_fragment(slug: &str) -> Option<Cascade> {
    PROCESSES
        .iter()
        .find(|p| p.slug == slug)
        .map(|p| parse_fragment(p.slug, p.toml, p.source_path))
}

/// All nozzle SKUs bundled for the named printer, in declaration
/// order. Empty vec for unknown printer slugs.
pub fn nozzle_skus_for(printer_slug: &str) -> Vec<&'static str> {
    NOZZLES
        .iter()
        .filter(|n| n.printer_slug == printer_slug)
        .map(|n| n.sku)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_fragment_parses() {
        for p in PRINTERS {
            assert!(
                load_printer_fragment(p.slug).is_some(),
                "printer fragment `{}` missing",
                p.slug,
            );
        }
        for n in NOZZLES {
            assert!(
                load_nozzle_fragment(n.printer_slug, n.sku).is_some(),
                "nozzle ({}, {}) missing",
                n.printer_slug,
                n.sku,
            );
        }
        for b in BEDS {
            assert!(
                load_bed_fragment(b.printer_slug, b.identity).is_some(),
                "bed ({}, {}) missing",
                b.printer_slug,
                b.identity,
            );
        }
        for f in FILAMENTS {
            assert!(load_filament_fragment(f.slug).is_some(), "filament `{}` missing", f.slug);
        }
        for p in PROCESSES {
            assert!(load_process_fragment(p.slug).is_some(), "process `{}` missing", p.slug);
        }
    }

    #[test]
    fn bambi_printer_fragment_carries_machine_envelope_not_nozzle_keys() {
        let cascade = load_printer_fragment("bambu-lab-a1-mini").expect("bambi printer");
        let rule = &cascade.rules[0];
        // Printer.toml should have machine globals but NOT per-extruder keys.
        assert!(rule.set.contains_key("printable_height"));
        assert!(rule.set.contains_key("machine_max_acceleration_x"));
        // nozzle_diameter is per-extruder → lives in nozzles/, NOT in printer.toml.
        assert!(
            !rule.set.contains_key("nozzle_diameter"),
            "nozzle_diameter must NOT be in printer.toml (lives in nozzles/<sku>.toml)",
        );
    }

    #[test]
    fn a1_mini_0_4_nozzle_carries_scalar_diameter() {
        let cascade = load_nozzle_fragment("bambu-lab-a1-mini", "0.4").expect("0.4 nozzle");
        let rule = &cascade.rules[0];
        let diameter = rule.set.get("nozzle_diameter").expect("nozzle_diameter present");
        // Scalar — not "0.4,0.4,0.4,0.4". The composer replicates for
        // multi-extruder printers.
        assert_eq!(diameter, "0.4");
    }

    #[test]
    fn u1_0_4_nozzle_is_also_scalar_despite_4_extruders() {
        // U1's leaf JSON had ["0.4","0.4","0.4","0.4"] — the converter
        // collapses the homogeneous array to scalar "0.4", and the
        // composer will replicate at slice time.
        let cascade = load_nozzle_fragment("snapmaker-u1", "0.4").expect("U1 0.4 nozzle");
        let rule = &cascade.rules[0];
        let diameter = rule.set.get("nozzle_diameter").expect("nozzle_diameter present");
        assert_eq!(diameter, "0.4", "U1's 0.4 nozzle scalar must equal 0.4");
    }

    #[test]
    fn supertack_bed_carries_curr_bed_type_enum_value() {
        let cascade = load_bed_fragment("bambu-lab-a1-mini", "Supertack Plate")
            .expect("supertack bed");
        let rule = &cascade.rules[0];
        assert_eq!(rule.set.get("curr_bed_type").map(String::as_str), Some("Supertack Plate"));
        assert_eq!(rule.set.get("identity").map(String::as_str), Some("Supertack Plate"));
    }

    #[test]
    fn bundled_beds_for_printer_lists_full_a1_mini_range() {
        let beds = bundled_beds_for_printer("bambu-lab-a1-mini");
        assert_eq!(
            beds,
            vec![
                "Cool Plate",
                "Textured PEI Plate",
                "High Temp Plate",
                "Engineering Plate",
                "Supertack Plate",
            ],
        );
        assert_eq!(bundled_beds_for_printer("snapmaker-u1"), vec!["Textured PEI Plate"]);
        assert!(bundled_beds_for_printer("ghost-printer").is_empty());
    }

    #[test]
    fn nozzle_skus_for_returns_declaration_order() {
        let bambu = nozzle_skus_for("bambu-lab-a1-mini");
        assert_eq!(bambu, vec!["0.2", "0.4", "0.6", "0.8"]);
        let u1 = nozzle_skus_for("snapmaker-u1");
        assert_eq!(u1, vec!["0.4", "0.6"]);
        assert!(nozzle_skus_for("ghost-printer").is_empty());
    }

    #[test]
    fn unknown_slugs_return_none() {
        assert!(load_printer_fragment("ghost").is_none());
        assert!(load_nozzle_fragment("bambu-lab-a1-mini", "9.9").is_none());
        assert!(load_bed_fragment("ghost", "Cool Plate").is_none());
        assert!(load_bed_fragment("bambu-lab-a1-mini", "Ghost Plate").is_none());
        assert!(load_filament_fragment("ghost").is_none());
        assert!(load_process_fragment("ghost").is_none());
    }
}
