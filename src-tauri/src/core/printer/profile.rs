//! Printer profile — declarative description of a physical printer.
//!
//! Loaded once per active printer (from a JSON file shipped under
//! `profiles/printers/` or authored by the user). The cascade
//! resolver reads `model`, `slot_count`, and per-toolhead config via
//! `Context::predicate_value`; the scene-state code (Phase 2) reads
//! `build_volume` + `exclusion_zones`; the driver layer (Phase 5)
//! reads per-toolhead config for sync-on-send decisions.

use serde::{Deserialize, Serialize};

/// Declarative description of a physical printer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrinterProfile {
    /// Human-readable model name. Surfaced as `printer.model` in
    /// cascade predicates.
    pub model: String,

    /// Manufacturer name (e.g. `"Bambu Lab"`, `"Snapmaker"`).
    /// Drives the add-printer modal's grouping + brand-tinted
    /// cards. `#[serde(default)]` so legacy profiles without the
    /// field still load — they render under an empty-brand group.
    #[serde(default)]
    pub brand: String,

    /// Short brand mark for the modal's profile cards + chips
    /// (e.g. `"B"` for Bambu Lab, `"S"` for Snapmaker). One or
    /// two characters; the design treats it as a glyph in a
    /// rounded square. `#[serde(default)]` for backward compat.
    #[serde(default)]
    pub brand_short: String,

    /// Number of filament slots (AMS slots on Bambu, toolheads on
    /// Snapmaker U1). Surfaced as `printer.slot_count`.
    pub slot_count: usize,

    /// Maximum number of AMS-style swap units this printer
    /// accepts. `0` for printers with no AMS support (direct-feed
    /// only, e.g. Snapmaker U1). `1` for single-AMS printers
    /// (A1 mini + AMS Lite). Higher for stackable AMS hosts
    /// (X1C/P1S can chain up to 4 AMS units). Drives the modal's
    /// AMS picker — when `0`, the picker is hidden entirely.
    #[serde(default)]
    pub ams_max: u32,

    /// User-visible AMS family name (`"AMS Lite"`, `"AMS"`,
    /// `"AMS 2 Pro"`). `None` when the printer has no AMS
    /// support (`ams_max == 0`). The modal renders this as
    /// `"<ams_type> configuration"`.
    #[serde(default)]
    pub ams_type: Option<String>,

    /// Build-plate identities this printer can target. Derived from
    /// the bed fragments bundled at
    /// `profiles/vendor/<vendor>/printer/<slug>/beds/<id>.toml` — the
    /// registry populates this in `bundled_catalog`/`lookup` from
    /// `bundled_beds_for_printer(slug)`. `#[serde(default)]` so
    /// printer TOMLs don't need to repeat the list.
    #[serde(default)]
    pub supported_build_plates: Vec<String>,

    /// One entry per physical toolhead. For Bambu A1 mini (AMS-fed
    /// single nozzle), `toolheads.len() == 1` and `slot_count == 4`.
    /// For Snapmaker U1 (toolchanger), `toolheads.len() == 4` and
    /// `slot_count == 4`.
    pub toolheads: Vec<Toolhead>,

    pub build_volume: BoundingBox,

    /// Areas that must not be printed in (parking bays, accessory
    /// mounts, calibration squares). Phase 2's scene state uses
    /// these for layout validation.
    #[serde(default)]
    pub exclusion_zones: Vec<BoundingBox>,
}

/// A single toolhead's hardware config. Per-extruder cascade
/// expansion (when Phase 1 grows beyond bed_temp) reads these.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Toolhead {
    pub nozzle_diameter: f64,
    pub hotend_type: String,
    pub max_temp: f64,
    /// Slot indices this toolhead can pull filament from. For a
    /// dual-feed AMS-Lite-style printer this would be `[0]`
    /// (single toolhead reads from any of the 4 slots through the
    /// AMS feed). For a U1 toolchanger this would be `[i]` (each
    /// toolhead bound to one slot).
    pub slot_indices: Vec<usize>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct BoundingBox {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bambu_a1_mini() -> PrinterProfile {
        PrinterProfile {
            model: "Bambu A1 mini".into(),
            brand: "Bambu Lab".into(),
            brand_short: "B".into(),
            slot_count: 4,
            ams_max: 1,
            ams_type: Some("AMS Lite".into()),
            supported_build_plates: vec![
                "Cool Plate".into(),
                "Textured PEI Plate".into(),
                "High Temp Plate".into(),
                "Engineering Plate".into(),
                "Supertack Plate".into(),
            ],
            toolheads: vec![Toolhead {
                nozzle_diameter: 0.4,
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
                slot_indices: vec![0, 1, 2, 3],
            }],
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [180.0, 180.0, 180.0],
            },
            exclusion_zones: vec![],
        }
    }

    #[test]
    fn a1_mini_shape_round_trips_toml() {
        let p = bambu_a1_mini();
        let text = toml::to_string(&p).expect("serialize");
        let parsed: PrinterProfile = toml::from_str(&text).expect("deserialize");
        assert_eq!(parsed.model, "Bambu A1 mini");
        assert_eq!(parsed.slot_count, 4);
        assert_eq!(parsed.toolheads.len(), 1);
        assert_eq!(parsed.toolheads[0].nozzle_diameter, 0.4);
        assert_eq!(parsed.supported_build_plates.len(), 5);
    }
}
