//! Printer profile — declarative description of a physical printer.
//!
//! Loaded once per active printer (from a JSON file shipped under
//! `profiles/printers/` or authored by the user). The cascade
//! resolver reads `model` and per-toolhead config via
//! `Context::predicate_value`; the scene-state code (Phase 2) reads
//! `build_volume` + `exclusion_zones`; the driver layer (Phase 5)
//! reads per-toolhead config for sync-on-send decisions.

use serde::{Deserialize, Serialize};

/// Declarative description of a physical printer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrinterProfile {
    /// Human-readable model name. Surfaced as `printer.model` in
    /// cascade predicates. **Derived** — hydrated by the registry
    /// from `machine.toml::printer_model` (the cascade scalar that
    /// drives every `when.printer.model = …` predicate). `model.toml`
    /// no longer carries it; the registry's `hydrate_profile`
    /// populates this field at load time. `#[serde(default)]` so
    /// the envelope parses cleanly.
    #[serde(default)]
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
    /// `profiles/<vendor>/printer/<slug>/beds/<id>.toml` — the
    /// registry populates this in `bundled_catalog`/`lookup` from
    /// `bundled_beds_for_printer(slug)`. `#[serde(default)]` so
    /// printer TOMLs don't need to repeat the list.
    #[serde(default)]
    pub supported_build_plates: Vec<String>,

    /// Nozzle diameters the printer ships fragments for, in
    /// declaration order (e.g. `["0.2", "0.4", "0.6", "0.8"]` for
    /// the A1 mini, `["0.4", "0.6"]` for the U1). Stored as
    /// **string symbols**, not floats — diameter is an identifier
    /// the picker filters by exact match (and the nozzle.toml
    /// filename derives from), never a quantity we arithmetic on.
    /// Hydrated by `registry::hydrate_profile` from
    /// `profile_library::nozzle_skus_for(slug)`; `#[serde(default)]`
    /// so model.toml doesn't have to repeat it.
    #[serde(default)]
    pub available_nozzle_diameters: Vec<String>,

    /// Default `curr_bed_type` enum value Orca's upstream
    /// `machine_model` JSON declares for this printer (e.g.
    /// `"Textured PEI Plate"` for both the A1 mini and the U1).
    /// Seeded into `model.toml` by `import_machine_profile.py`
    /// from the model JSON's `default_bed_type` field. `None` for
    /// printers whose upstream profile omits the field.
    #[serde(default)]
    pub default_bed: Option<String>,

    /// One entry per physical toolhead. `toolheads.len()` is the
    /// canonical extruder count: 1 for AMS-fed printers (Bambu A1
    /// mini), N for toolchangers (Snapmaker U1). Combined with
    /// `ams_max`, it distinguishes the multi-material flavor:
    /// `toolheads.len() == 1 && ams_max > 0` is AMS-style;
    /// `toolheads.len() > 1` is toolchanger.
    pub toolheads: Vec<Toolhead>,

    /// Which driver implementation (if any) talks to this printer.
    /// `None` means n3o has no driver for it — the picker shows a
    /// "not configured" hint and the Connection tab in the
    /// settings modal is hidden. **Authored** in each printer's
    /// `model.toml` (`driver_kind = "bambu" | "u1"`); the registry
    /// carries it through unchanged. `#[serde(default)]` so a
    /// printer that ships without a driver can omit the field and
    /// resolve to `None`. The connection setters
    /// (`set_instance_connection` / `update_instance`) enforce that a
    /// saved `ConnectionInfo` variant matches this kind.
    #[serde(default)]
    pub driver_kind: Option<crate::core::driver::traits::DriverKind>,

    /// Hydrated by the registry from the machine cascade's
    /// `printable_area` (XY corners polygon) + `printable_height`.
    /// `#[serde(default)]` so model.toml doesn't repeat what the
    /// cascade already declares — the zero AABB lives just long
    /// enough for `hydrate_profile` to replace it.
    #[serde(default)]
    pub build_volume: BoundingBox,

    /// Areas that must not be printed in (parking bays, accessory
    /// mounts, calibration squares). Phase 2's scene state uses
    /// these for layout validation.
    #[serde(default)]
    pub exclusion_zones: Vec<BoundingBox>,
}

impl PrinterProfile {
    /// True when this printer can host multiple filament slots
    /// simultaneously — either via multiple physical toolheads
    /// (toolchangers, U1) or via an AMS feeding a single toolhead
    /// (Bambu A1 mini + AMS Lite). Single-toolhead printers without
    /// AMS support return false.
    pub fn has_multiple_slots(&self) -> bool {
        self.toolheads.len() > 1 || self.ams_max > 0
    }
}

/// A single toolhead's hardware config. The nozzle is a swappable
/// consumable — `default_nozzle_diameter` is just the SKU the printer
/// ships with / what `create_instance` seeds onto a fresh instance.
/// The runtime `ExtruderState.installed_nozzle` holds the current
/// nozzle; this field is *not* consulted at slice time.
///
/// `default_nozzle_diameter` is a string symbol ("0.4"), matching
/// the on-disk nozzle.toml filename. See
/// `PrinterProfile.available_nozzle_diameters` for the rationale.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Toolhead {
    pub default_nozzle_diameter: String,
    /// Hotend material descriptor (`"stainless_steel"`,
    /// `"hardened_steel"`, …). **Derived** — hydrated by the
    /// registry from `nozzles/<default_nozzle_diameter>.toml::nozzle_type`,
    /// since the nozzle profile is the per-SKU source of truth for
    /// that string. `model.toml` no longer carries it.
    /// `#[serde(default)]` so the toolhead block parses cleanly.
    #[serde(default)]
    pub hotend_type: String,
    pub max_temp: f64,
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
            model: "Bambu Lab A1 mini".into(),
            brand: "Bambu Lab".into(),
            brand_short: "B".into(),
            ams_max: 1,
            ams_type: Some("AMS Lite".into()),
            default_bed: Some("Textured PEI Plate".into()),
            supported_build_plates: vec![
                "Cool Plate".into(),
                "Textured PEI Plate".into(),
                "High Temp Plate".into(),
                "Engineering Plate".into(),
                "Supertack Plate".into(),
            ],
            available_nozzle_diameters: vec![
                "0.2".into(),
                "0.4".into(),
                "0.6".into(),
                "0.8".into(),
            ],
            toolheads: vec![Toolhead {
                default_nozzle_diameter: "0.4".into(),
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
            }],
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [180.0, 180.0, 180.0],
            },
            exclusion_zones: vec![],
            driver_kind: Some(crate::core::driver::traits::DriverKind::Bambu),
        }
    }

    #[test]
    fn a1_mini_shape_round_trips_toml() {
        let p = bambu_a1_mini();
        let text = toml::to_string(&p).expect("serialize");
        let parsed: PrinterProfile = toml::from_str(&text).expect("deserialize");
        assert_eq!(parsed.model, "Bambu Lab A1 mini");
        assert_eq!(parsed.toolheads.len(), 1);
        assert_eq!(parsed.toolheads[0].default_nozzle_diameter, "0.4");
        assert_eq!(parsed.supported_build_plates.len(), 5);
    }
}
