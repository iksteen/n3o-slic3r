//! Per-key printer-settings category + per-extruder flag, scraped from
//! OrcaSlicer.
//!
//! AUTO-GENERATED — do not edit by hand. Regenerate with
//! `scripts/scrape_option_printer_pages.py` after pulling new upstream
//! OrcaSlicer source.
//!
//! Category (from `src/slic3r/GUI/Tab.cpp` `TabPrinter`): the
//! `add_options_page` title the key appears under (Basic information,
//! Machine G-code, …) for machine-wide options, or the `new_optgroup`
//! title (Retraction, Z-Hop, …) for keys in the extruder-page loop —
//! printer options carry no libslic3r `category` of their own. Keys absent
//! from the table return `None`; callers fall back to an "Other" bucket.
//!
//! Per-extruder set (from `src/libslic3r/PrintConfig.cpp`
//! `m_extruder_option_keys`): the authoritative list of options sized to
//! the extruder count. Sourced from the data model, not the GUI, because
//! Orca's extruder-page widgets omit some members (e.g. `extruder_colour`,
//! whose widget is commented out). These render one tab per toolhead.

const PRINTER_PAGES: &[(&str, &str)] = &[
    ("adaptive_bed_mesh_margin", "Basic information"),
    ("auxiliary_fan", "Basic information"),
    ("bbl_use_printhost", "Basic information"),
    ("bed_exclude_area", "Basic information"),
    ("bed_mesh_max", "Basic information"),
    ("bed_mesh_min", "Basic information"),
    ("bed_mesh_probe_distance", "Basic information"),
    ("bed_temperature_formula", "Multimaterial"),
    ("before_layer_change_gcode", "Machine G-code"),
    ("best_object_pos", "Basic information"),
    ("change_extrusion_role_gcode", "Machine G-code"),
    ("change_filament_gcode", "Machine G-code"),
    ("cooling_tube_length", "Multimaterial"),
    ("cooling_tube_retraction", "Multimaterial"),
    ("deretraction_speed", "Retraction"),
    ("disable_m73", "Basic information"),
    ("emit_machine_limits_to_gcode", "Motion ability"),
    ("enable_filament_ramming", "Multimaterial"),
    ("enable_power_loss_recovery", "Basic information"),
    ("extra_loading_move", "Multimaterial"),
    ("extruder_clearance_height_to_lid", "Basic information"),
    ("extruder_clearance_height_to_rod", "Basic information"),
    ("extruder_clearance_radius", "Basic information"),
    ("extruder_offset", "Position"),
    ("extruder_printable_area", "Basic information"),
    ("extruder_printable_height", "Basic information"),
    ("fan_kickstart", "Basic information"),
    ("fan_speedup_overhangs", "Basic information"),
    ("fan_speedup_time", "Basic information"),
    ("file_start_gcode", "Machine G-code"),
    ("gcode_flavor", "Basic information"),
    ("high_current_on_filament_swap", "Multimaterial"),
    ("input_shaping_damp_x", "Motion ability"),
    ("input_shaping_damp_y", "Motion ability"),
    ("input_shaping_emit", "Motion ability"),
    ("input_shaping_freq_x", "Motion ability"),
    ("input_shaping_freq_y", "Motion ability"),
    ("input_shaping_type", "Motion ability"),
    ("layer_change_gcode", "Machine G-code"),
    ("long_retractions_when_cut", "Retraction when switching material"),
    ("machine_end_gcode", "Machine G-code"),
    ("machine_load_filament_time", "Multimaterial"),
    ("machine_pause_gcode", "Machine G-code"),
    ("machine_start_gcode", "Machine G-code"),
    ("machine_tool_change_time", "Multimaterial"),
    ("machine_unload_filament_time", "Multimaterial"),
    ("manual_filament_change", "Multimaterial"),
    ("max_layer_height", "Layer height limits"),
    ("max_resonance_avoidance_speed", "Motion ability"),
    ("min_layer_height", "Layer height limits"),
    ("min_resonance_avoidance_speed", "Motion ability"),
    ("nozzle_diameter", "Basic information"),
    ("nozzle_hrc", "Basic information"),
    ("nozzle_type", "Basic information"),
    ("nozzle_volume", "Basic information"),
    ("nozzle_volume_type", "Basic information"),
    ("parallel_printheads_count", "Basic information"),
    ("parking_pos_retraction", "Multimaterial"),
    ("part_cooling_fan_min_pwm", "Basic information"),
    ("pellet_modded_printer", "Basic information"),
    ("preferred_orientation", "Basic information"),
    ("printable_area", "Basic information"),
    ("printable_height", "Basic information"),
    ("printer_notes", "Notes"),
    ("printer_structure", "Basic information"),
    ("printing_by_object_gcode", "Machine G-code"),
    ("purge_in_prime_tower", "Multimaterial"),
    ("resonance_avoidance", "Motion ability"),
    ("retract_before_wipe", "Retraction"),
    ("retract_length_toolchange", "Retraction when switching material"),
    ("retract_lift_above", "Z-Hop"),
    ("retract_lift_below", "Z-Hop"),
    ("retract_lift_enforce", "Z-Hop"),
    ("retract_restart_extra", "Retraction"),
    ("retract_restart_extra_toolchange", "Retraction when switching material"),
    ("retract_when_changing_layer", "Retraction"),
    ("retraction_distances_when_cut", "Retraction when switching material"),
    ("retraction_length", "Retraction"),
    ("retraction_minimum_travel", "Retraction"),
    ("retraction_speed", "Retraction"),
    ("scan_first_layer", "Basic information"),
    ("single_extruder_multi_material", "Multimaterial"),
    ("spaghetti_detector", "Basic information"),
    ("support_air_filtration", "Basic information"),
    ("support_chamber_temp_control", "Basic information"),
    ("support_multi_bed_types", "Basic information"),
    ("template_custom_gcode", "Machine G-code"),
    ("thumbnails", "Basic information"),
    ("thumbnails_format", "Basic information"),
    ("time_cost", "Basic information"),
    ("time_lapse_gcode", "Machine G-code"),
    ("tool_change_on_wipe_tower", "Multimaterial"),
    ("travel_slope", "Z-Hop"),
    ("use_firmware_retraction", "Basic information"),
    ("use_relative_e_distances", "Basic information"),
    ("wipe", "Retraction"),
    ("wipe_distance", "Retraction"),
    ("wipe_tower_type", "Multimaterial"),
    ("wrapping_detection_gcode", "Machine G-code"),
    ("wrapping_exclude_area", "Basic information"),
    ("z_hop", "Z-Hop"),
    ("z_hop_types", "Z-Hop"),
    ("z_offset", "Basic information"),
];

const PRINTER_SUBGROUPS: &[(&str, &str)] = &[
    ("adaptive_bed_mesh_margin", "Adaptive bed mesh"),
    ("auxiliary_fan", "Accessory"),
    ("bbl_use_printhost", "Advanced"),
    ("bed_exclude_area", "Printable space"),
    ("bed_mesh_max", "Adaptive bed mesh"),
    ("bed_mesh_min", "Adaptive bed mesh"),
    ("bed_mesh_probe_distance", "Adaptive bed mesh"),
    ("bed_temperature_formula", "Single extruder multi-material setup"),
    ("before_layer_change_gcode", "Before layer change G-code"),
    ("best_object_pos", "Printable space"),
    ("change_extrusion_role_gcode", "Change extrusion role G-code"),
    ("change_filament_gcode", "Change filament G-code"),
    ("cooling_tube_length", "Single extruder multi-material parameters"),
    ("cooling_tube_retraction", "Single extruder multi-material parameters"),
    ("disable_m73", "Advanced"),
    ("emit_machine_limits_to_gcode", "Advanced"),
    ("enable_filament_ramming", "Wipe tower"),
    ("enable_power_loss_recovery", "Advanced"),
    ("extra_loading_move", "Single extruder multi-material parameters"),
    ("extruder_clearance_height_to_lid", "Extruder Clearance"),
    ("extruder_clearance_height_to_rod", "Extruder Clearance"),
    ("extruder_clearance_radius", "Extruder Clearance"),
    ("fan_kickstart", "Cooling Fan"),
    ("fan_speedup_overhangs", "Cooling Fan"),
    ("fan_speedup_time", "Cooling Fan"),
    ("file_start_gcode", "File header G-code"),
    ("gcode_flavor", "Advanced"),
    ("high_current_on_filament_swap", "Single extruder multi-material parameters"),
    ("input_shaping_damp_x", "Resonance Compensation"),
    ("input_shaping_damp_y", "Resonance Compensation"),
    ("input_shaping_emit", "Resonance Compensation"),
    ("input_shaping_freq_x", "Resonance Compensation"),
    ("input_shaping_freq_y", "Resonance Compensation"),
    ("input_shaping_type", "Resonance Compensation"),
    ("layer_change_gcode", "Layer change G-code"),
    ("machine_end_gcode", "Machine end G-code"),
    ("machine_load_filament_time", "Advanced"),
    ("machine_pause_gcode", "Pause G-code"),
    ("machine_start_gcode", "Machine start G-code"),
    ("machine_tool_change_time", "Advanced"),
    ("machine_unload_filament_time", "Advanced"),
    ("manual_filament_change", "Single extruder multi-material setup"),
    ("max_resonance_avoidance_speed", "Resonance Compensation"),
    ("min_resonance_avoidance_speed", "Resonance Compensation"),
    ("nozzle_hrc", "Accessory"),
    ("nozzle_type", "Accessory"),
    ("parallel_printheads_count", "Printable space"),
    ("parking_pos_retraction", "Single extruder multi-material parameters"),
    ("part_cooling_fan_min_pwm", "Cooling Fan"),
    ("pellet_modded_printer", "Advanced"),
    ("preferred_orientation", "Printable space"),
    ("printable_area", "Printable space"),
    ("printable_height", "Printable space"),
    ("printer_notes", "Notes"),
    ("printer_structure", "Advanced"),
    ("printing_by_object_gcode", "Printing by object G-code"),
    ("purge_in_prime_tower", "Wipe tower"),
    ("resonance_avoidance", "Resonance Compensation"),
    ("scan_first_layer", "Advanced"),
    ("single_extruder_multi_material", "Single extruder multi-material setup"),
    ("spaghetti_detector", "Advanced"),
    ("support_air_filtration", "Accessory"),
    ("support_chamber_temp_control", "Accessory"),
    ("support_multi_bed_types", "Printable space"),
    ("template_custom_gcode", "Template Custom G-code"),
    ("thumbnails", "Advanced"),
    ("thumbnails_format", "Advanced"),
    ("time_cost", "Advanced"),
    ("time_lapse_gcode", "Timelapse G-code"),
    ("tool_change_on_wipe_tower", "Wipe tower"),
    ("use_firmware_retraction", "Advanced"),
    ("use_relative_e_distances", "Advanced"),
    ("wipe_tower_type", "Wipe tower"),
    ("wrapping_detection_gcode", "Clumping Detection G-code"),
    ("wrapping_exclude_area", "Advanced"),
    ("z_offset", "Printable space"),
];

const PER_EXTRUDER: &[&str] = &[
    "default_filament_profile",
    "default_nozzle_volume_type",
    "deretraction_speed",
    "extruder_colour",
    "extruder_offset",
    "extruder_printable_height",
    "extruder_type",
    "long_retractions_when_cut",
    "max_layer_height",
    "min_layer_height",
    "nozzle_diameter",
    "nozzle_flush_dataset",
    "nozzle_type",
    "nozzle_volume",
    "retract_before_wipe",
    "retract_length_toolchange",
    "retract_lift_above",
    "retract_lift_below",
    "retract_lift_enforce",
    "retract_restart_extra",
    "retract_restart_extra_toolchange",
    "retract_when_changing_layer",
    "retraction_distances_when_cut",
    "retraction_length",
    "retraction_minimum_travel",
    "retraction_speed",
    "travel_slope",
    "wipe",
    "wipe_distance",
    "z_hop",
    "z_hop_types",
];

/// The printer-settings category the option key appears under in Orca's
/// `TabPrinter` (page for machine-wide keys, optgroup for per-extruder
/// keys), or `None` for keys not laid out there.
pub fn printer_page_of(key: &str) -> Option<&'static str> {
    PRINTER_PAGES
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| PRINTER_PAGES[i].1)
}

/// The optgroup (sub-section within a page) a machine-wide option appears
/// under — e.g. "Printable space" under the "Basic information" page. The
/// panel renders these as sub-headers within the page. `None` for keys with
/// no sub-group (per-extruder keys, or keys not laid out in Tab.cpp).
pub fn printer_subgroup_of(key: &str) -> Option<&'static str> {
    PRINTER_SUBGROUPS
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| PRINTER_SUBGROUPS[i].1)
}

/// True if the option is laid out per-extruder (one value per toolhead)
/// in Orca's `TabPrinter` — the set the per-extruder UI tabs surface.
pub fn is_per_extruder(key: &str) -> bool {
    PER_EXTRUDER.binary_search(&key).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_for_binary_search() {
        let mut last = "";
        for (key, _) in PRINTER_PAGES {
            assert!(*key > last, "PRINTER_PAGES must be sorted; {key} <= {last}");
            last = key;
        }
        let mut last = "";
        for (key, _) in PRINTER_SUBGROUPS {
            assert!(*key > last, "PRINTER_SUBGROUPS must be sorted; {key} <= {last}");
            last = key;
        }
        let mut last = "";
        for key in PER_EXTRUDER {
            assert!(*key > last, "PER_EXTRUDER must be sorted; {key} <= {last}");
            last = key;
        }
    }

    #[test]
    fn known_keys_map_to_their_orca_category() {
        assert_eq!(printer_page_of("gcode_flavor"), Some("Basic information"));
        assert_eq!(printer_page_of("machine_start_gcode"), Some("Machine G-code"));
        assert_eq!(printer_page_of("retraction_length"), Some("Retraction"));
        // Sub-group within a page: z_offset sits under "Printable space".
        assert_eq!(printer_subgroup_of("z_offset"), Some("Printable space"));
        assert_eq!(printer_subgroup_of("gcode_flavor"), Some("Advanced"));
    }

    #[test]
    fn per_extruder_flag_matches_libslic3r_set() {
        assert!(is_per_extruder("retraction_length"));
        assert!(is_per_extruder("z_hop"));
        assert!(is_per_extruder("nozzle_diameter"));
        // In libslic3r's set even though Orca's extruder-page widget for
        // it is commented out.
        assert!(is_per_extruder("extruder_colour"));
        // Machine-wide keys are not per-extruder.
        assert!(!is_per_extruder("gcode_flavor"));
        assert!(!is_per_extruder("machine_start_gcode"));
    }

    #[test]
    fn unknown_key_returns_none() {
        assert!(printer_page_of("totally_made_up_option").is_none());
        assert!(!is_per_extruder("totally_made_up_option"));
    }
}
