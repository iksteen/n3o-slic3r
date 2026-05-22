//! Translation manifest — logical cascade keys ↔ libslic3r option keys.
//!
//! Maintained as a Rust data structure rather than a TOML side-file
//! for now; we own the truth set and code-review every change rather
//! than letting profile authors edit it. A future move to a separate
//! file is fine if/when the manifest grows past ~200 entries.
//!
//! Three categories of entries:
//!
//! - **Identity** is the default. Logical key `layer_height` maps 1:1
//!   to libslic3r `layer_height`. No manifest entry needed.
//! - **Dimensional** entries enumerate the libslic3r keys that one
//!   logical key expands into. Currently just `bed_temp` → 12 per-plate
//!   keys (see `crate::core::schema::BED_TEMP_KEYS`). The adapter
//!   resolves the cascade against each plate type and writes the
//!   per-plate value.
//! - **Drop** entries silently discard keys that are OrcaSlicer-only
//!   metadata (no libslic3r equivalent). Sourced from PR-0.5-1
//!   (67 keys discovered on the Bambu cascade) and PR-0.5-2 (13 keys
//!   on the Prusa cascade), with five Orca-side typos remapped to
//!   their correct spellings rather than dropped.

use std::collections::{HashMap, HashSet};

/// Curated list of OrcaSlicer-only keys we drop silently at adapt
/// time. Sourced from the PR-0.5-1 / PR-0.5-2 finding docs. Keys
/// here never make it into the `slic3r_ffi::Config`; the adapter
/// doesn't log a warning on each occurrence (these are expected).
pub const DROP_LIST: &[&str] = &[
    // PR-0.5-1 (Bambu A1 mini cascade) — Bambu firmware tuning
    "hotend_cooling_rate",
    "hotend_heating_rate",
    "machine_prepare_compensation_time",
    "machine_switch_extruder_time",
    "enable_pre_heating",
    "chamber_temperatures",
    // AMS / multi-color extensions (Bambu-side firmware codes)
    "filament_long_retractions_when_ec",
    "filament_retraction_distances_when_ec",
    "filament_scarf_gap",
    "filament_scarf_height",
    "filament_scarf_length",
    "filament_scarf_seam_type",
    "filament_prime_volume",
    "filament_ramming_travel_time",
    "filament_ramming_volumetric_speed",
    "filament_velocity_adaptation_factor",
    "override_filament_scarf_seam_setting",
    // Circle / hole compensation (Bambu firmware)
    "circle_compensation_manual_offset",
    "counter_coef_1",
    "counter_coef_2",
    "counter_coef_3",
    "counter_limit_max",
    "counter_limit_min",
    "hole_coef_1",
    "hole_coef_2",
    "hole_coef_3",
    "hole_limit_max",
    "hole_limit_min",
    "diameter_limit",
    "enable_circle_compensation",
    "apply_top_surface_compensation",
    // OrcaSlicer-fork-only process knobs
    "adaptive_layer_height",
    "layer_time_smoothing",
    "layer_time_smoothing_threshold",
    "slowdown_start_acc",
    "slowdown_start_height",
    "slowdown_start_speed",
    "slowdown_end_acc",
    "slowdown_end_height",
    "slowdown_end_speed",
    "prime_tower_lift_height",
    "prime_tower_lift_speed",
    "prime_tower_max_speed",
    "overhang_totally_speed",
    "pre_start_fan_time",
    "top_color_penetration_layers",
    "bottom_color_penetration_layers",
    "seam_slope_gap",
    "seam_placement_away_from_overhangs",
    "smooth_coefficient",
    "infill_rotate_step",
    "internal_bridge_support_thickness",
    "wall_infill_order",
    "vertical_shell_speed",
    "z_direction_outwall_speed_continuous",
    "detect_floating_vertical_shell",
    "enable_height_slowdown",
    "impact_strength_z",
    "locked_skin_infill_pattern",
    "locked_skeleton_infill_pattern",
    "filament_id",
    "smooth_plate_temp",
    "smooth_plate_temp_initial_layer",
    // PR-0.5-2 (Prusa XL cascade) extras
    "bed_type",
    "filament_load_time",
    "filament_unload_time",
    "tree_support_branch_diameter_double_wall",
    // Extruder clearance miscellany
    "extruder_clearance_dist_to_rod",
    "extruder_clearance_max_radius",
];

/// Five Orca-side typos discovered in PR-0.5-2's finding (in
/// OrcaSlicer's *own* profile JSONs). The adapter silently rewrites
/// them to the correct spelling before pushing into libslic3r.
///
/// These have no effect in OrcaSlicer either — libslic3r silently
/// drops unknown keys upstream — so remapping them recovers the
/// authors' intent.
pub const TYPO_REMAP: &[(&str, &str)] = &[
    ("detraction_speed", "deretraction_speed"),
    ("inital_layer_height", "initial_layer_height"),
    ("nozzle_temperature_intial_layer", "nozzle_temperature_initial_layer"),
    ("tree_support_bramch_diameter_angle", "tree_support_branch_diameter_angle"),
    // wall_infill_order is in DROP_LIST today; once libslic3r exposes
    // a canonical version, add the remap here.
];

/// Manifest lookup. Build once via `Manifest::build()` and pass to
/// the adapter. Provides O(1) drop-list / typo-remap checks.
pub struct Manifest {
    drop_set: HashSet<&'static str>,
    typo_map: HashMap<&'static str, &'static str>,
}

impl Manifest {
    pub fn build() -> Self {
        Self {
            drop_set: DROP_LIST.iter().copied().collect(),
            typo_map: TYPO_REMAP.iter().copied().collect(),
        }
    }

    /// True if `key` should be silently dropped at adapt time.
    pub fn is_dropped(&self, key: &str) -> bool {
        self.drop_set.contains(key)
    }

    /// If `key` is an Orca-side typo, return the canonical libslic3r
    /// spelling. Otherwise `None`.
    pub fn typo_remap(&self, key: &str) -> Option<&'static str> {
        self.typo_map.get(key).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_list_contains_expected_entries() {
        let m = Manifest::build();
        // Spot-check a handful from each PR-0.5 finding category.
        assert!(m.is_dropped("hotend_cooling_rate"), "Bambu firmware tuning");
        assert!(m.is_dropped("filament_scarf_height"), "AMS extension");
        assert!(m.is_dropped("counter_coef_1"), "circle compensation");
        assert!(m.is_dropped("adaptive_layer_height"), "fork-only");
        assert!(m.is_dropped("smooth_plate_temp"), "Bambu plate variant");
        assert!(m.is_dropped("bed_type"), "Prusa cascade extra");
    }

    #[test]
    fn typo_remap_recovers_authors_intent() {
        let m = Manifest::build();
        assert_eq!(m.typo_remap("detraction_speed"), Some("deretraction_speed"));
        assert_eq!(
            m.typo_remap("inital_layer_height"),
            Some("initial_layer_height")
        );
        assert_eq!(
            m.typo_remap("nozzle_temperature_intial_layer"),
            Some("nozzle_temperature_initial_layer")
        );
        assert_eq!(m.typo_remap("layer_height"), None, "valid key not remapped");
    }
}
