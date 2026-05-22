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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterProfile {
    /// Human-readable model name. Surfaced as `printer.model` in
    /// cascade predicates.
    pub model: String,

    /// Number of filament slots (AMS slots on Bambu, toolheads on
    /// Snapmaker U1). Surfaced as `printer.slot_count`.
    pub slot_count: usize,

    /// Build-plate identities this printer can target. Reference the
    /// matching `profiles/plates/<identity>.json` files. Surfaced as
    /// a constraint for predicate validation (PR-1-2) but not as a
    /// predicate dimension itself.
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            slot_count: 4,
            supported_build_plates: vec![
                "Cool".into(),
                "Textured PEI".into(),
                "Smooth PEI".into(),
                "Engineering".into(),
                "SuperTack".into(),
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
