//! Per-key printer/filament-settings category, scraped from OrcaSlicer.
//!
//! AUTO-GENERATED — do not edit by hand. Regenerate with
//! `scripts/scrape_option_printer_pages.py` after pulling new upstream
//! OrcaSlicer source.
//!
//! Printer + filament options carry no libslic3r `category` of their own;
//! their grouping lives in `src/slic3r/GUI/Tab.cpp` (`TabPrinter` /
//! `TabFilament`). Each table maps a key to the `add_options_page` title it
//! appears under (and `new_optgroup` sub-title). Keys absent from a table
//! return `None`; callers fall back to an "Other" bucket. `printer_page_of`
//! / `filament_page_of` being `Some` is also the "Orca lays out an editor
//! for this key" signal the machine + filament panels gate visibility on.
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
    ("machine_max_acceleration_e", "Motion ability"),
    ("machine_max_acceleration_extruding", "Motion ability"),
    ("machine_max_acceleration_retracting", "Motion ability"),
    ("machine_max_acceleration_travel", "Motion ability"),
    ("machine_max_acceleration_x", "Motion ability"),
    ("machine_max_acceleration_y", "Motion ability"),
    ("machine_max_acceleration_z", "Motion ability"),
    ("machine_max_jerk_e", "Motion ability"),
    ("machine_max_jerk_x", "Motion ability"),
    ("machine_max_jerk_y", "Motion ability"),
    ("machine_max_jerk_z", "Motion ability"),
    ("machine_max_junction_deviation", "Motion ability"),
    ("machine_max_speed_e", "Motion ability"),
    ("machine_max_speed_x", "Motion ability"),
    ("machine_max_speed_y", "Motion ability"),
    ("machine_max_speed_z", "Motion ability"),
    ("machine_min_extruding_rate", "Motion ability"),
    ("machine_min_travel_rate", "Motion ability"),
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
    ("use_3mf", "Basic information"),
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
    ("machine_max_acceleration_e", "Acceleration limitation"),
    ("machine_max_acceleration_extruding", "Acceleration limitation"),
    ("machine_max_acceleration_retracting", "Acceleration limitation"),
    ("machine_max_acceleration_travel", "Acceleration limitation"),
    ("machine_max_acceleration_x", "Acceleration limitation"),
    ("machine_max_acceleration_y", "Acceleration limitation"),
    ("machine_max_acceleration_z", "Acceleration limitation"),
    ("machine_max_jerk_e", "Jerk limitation"),
    ("machine_max_jerk_x", "Jerk limitation"),
    ("machine_max_jerk_y", "Jerk limitation"),
    ("machine_max_jerk_z", "Jerk limitation"),
    ("machine_max_junction_deviation", "Jerk limitation"),
    ("machine_max_speed_e", "Speed limitation"),
    ("machine_max_speed_x", "Speed limitation"),
    ("machine_max_speed_y", "Speed limitation"),
    ("machine_max_speed_z", "Speed limitation"),
    ("machine_min_extruding_rate", "Minimum feedrates"),
    ("machine_min_travel_rate", "Minimum feedrates"),
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
    ("use_3mf", "Advanced"),
    ("use_firmware_retraction", "Advanced"),
    ("use_relative_e_distances", "Advanced"),
    ("wipe_tower_type", "Wipe tower"),
    ("wrapping_detection_gcode", "Clumping Detection G-code"),
    ("wrapping_exclude_area", "Advanced"),
    ("z_offset", "Printable space"),
];

const FILAMENT_PAGES: &[(&str, &str)] = &[
    ("activate_air_filtration", "Cooling"),
    ("activate_air_filtration_during_print", "Cooling"),
    ("activate_air_filtration_on_completion", "Cooling"),
    ("activate_chamber_temp_control", "Filament"),
    ("adaptive_pressure_advance", "Filament"),
    ("adaptive_pressure_advance_bridges", "Filament"),
    ("adaptive_pressure_advance_model", "Filament"),
    ("adaptive_pressure_advance_overhangs", "Filament"),
    ("additional_cooling_fan_speed", "Cooling"),
    ("chamber_minimal_temperature", "Filament"),
    ("chamber_temperature", "Filament"),
    ("close_fan_the_first_x_layers", "Cooling"),
    ("compatible_printers", "Dependencies"),
    ("compatible_printers_condition", "Dependencies"),
    ("compatible_prints", "Dependencies"),
    ("compatible_prints_condition", "Dependencies"),
    ("complete_print_exhaust_fan_speed", "Cooling"),
    ("cool_plate_temp", "Filament"),
    ("cool_plate_temp_initial_layer", "Filament"),
    ("default_filament_colour", "Filament"),
    ("dont_slow_down_outer_wall", "Cooling"),
    ("during_print_exhaust_fan_speed", "Cooling"),
    ("enable_overhang_bridge_fan", "Cooling"),
    ("enable_pressure_advance", "Filament"),
    ("eng_plate_temp", "Filament"),
    ("eng_plate_temp_initial_layer", "Filament"),
    ("fan_cooling_layer_time", "Cooling"),
    ("fan_max_speed", "Cooling"),
    ("fan_min_speed", "Cooling"),
    ("filament_adaptive_volumetric_speed", "Filament"),
    ("filament_adhesiveness_category", "Filament"),
    ("filament_change_extrusion_role_gcode", "Advanced"),
    ("filament_change_length", "Filament"),
    ("filament_colour", "Filament"),
    ("filament_cooling_final_speed", "Multimaterial"),
    ("filament_cooling_initial_speed", "Multimaterial"),
    ("filament_cooling_moves", "Multimaterial"),
    ("filament_cost", "Filament"),
    ("filament_density", "Filament"),
    ("filament_diameter", "Filament"),
    ("filament_end_gcode", "Advanced"),
    ("filament_flow_ratio", "Filament"),
    ("filament_flush_temp", "Multimaterial"),
    ("filament_flush_volumetric_speed", "Multimaterial"),
    ("filament_is_support", "Filament"),
    ("filament_loading_speed", "Multimaterial"),
    ("filament_loading_speed_start", "Multimaterial"),
    ("filament_max_volumetric_speed", "Filament"),
    ("filament_minimal_purge_on_wipe_tower", "Multimaterial"),
    ("filament_multitool_ramming", "Multimaterial"),
    ("filament_multitool_ramming_flow", "Multimaterial"),
    ("filament_multitool_ramming_volume", "Multimaterial"),
    ("filament_notes", "Notes"),
    ("filament_ramming_parameters", "Multimaterial"),
    ("filament_shrink", "Filament"),
    ("filament_shrinkage_compensation_z", "Filament"),
    ("filament_soluble", "Filament"),
    ("filament_stamping_distance", "Multimaterial"),
    ("filament_stamping_loading_speed", "Multimaterial"),
    ("filament_start_gcode", "Advanced"),
    ("filament_toolchange_delay", "Multimaterial"),
    ("filament_tower_interface_pre_extrusion_dist", "Multimaterial"),
    ("filament_tower_interface_pre_extrusion_length", "Multimaterial"),
    ("filament_tower_interface_print_temp", "Multimaterial"),
    ("filament_tower_interface_purge_volume", "Multimaterial"),
    ("filament_tower_ironing_area", "Multimaterial"),
    ("filament_type", "Filament"),
    ("filament_unloading_speed", "Multimaterial"),
    ("filament_unloading_speed_start", "Multimaterial"),
    ("filament_vendor", "Filament"),
    ("full_fan_speed_layer", "Cooling"),
    ("hot_plate_temp", "Filament"),
    ("hot_plate_temp_initial_layer", "Filament"),
    ("idle_temperature", "Filament"),
    ("internal_bridge_fan_speed", "Cooling"),
    ("ironing_fan_speed", "Cooling"),
    ("long_retractions_when_ec", "Multimaterial"),
    ("nozzle_temperature", "Filament"),
    ("nozzle_temperature_initial_layer", "Filament"),
    ("nozzle_temperature_range_high", "Filament"),
    ("nozzle_temperature_range_low", "Filament"),
    ("overhang_fan_speed", "Cooling"),
    ("overhang_fan_threshold", "Cooling"),
    ("pellet_flow_coefficient", "Filament"),
    ("pressure_advance", "Filament"),
    ("reduce_fan_stop_start_freq", "Cooling"),
    ("retraction_distances_when_ec", "Multimaterial"),
    ("slow_down_for_layer_cooling", "Cooling"),
    ("slow_down_layer_time", "Cooling"),
    ("slow_down_min_speed", "Cooling"),
    ("supertack_plate_temp", "Filament"),
    ("supertack_plate_temp_initial_layer", "Filament"),
    ("support_material_interface_fan_speed", "Cooling"),
    ("temperature_vitrification", "Filament"),
    ("textured_cool_plate_temp", "Filament"),
    ("textured_cool_plate_temp_initial_layer", "Filament"),
    ("textured_plate_temp", "Filament"),
    ("textured_plate_temp_initial_layer", "Filament"),
];

const FILAMENT_SUBGROUPS: &[(&str, &str)] = &[
    ("activate_air_filtration", "Exhaust fan"),
    ("activate_air_filtration_during_print", "Exhaust fan"),
    ("activate_air_filtration_on_completion", "Exhaust fan"),
    ("activate_chamber_temp_control", "Print chamber temperature"),
    ("adaptive_pressure_advance", "Flow ratio and Pressure Advance"),
    ("adaptive_pressure_advance_bridges", "Flow ratio and Pressure Advance"),
    ("adaptive_pressure_advance_model", "Flow ratio and Pressure Advance"),
    ("adaptive_pressure_advance_overhangs", "Flow ratio and Pressure Advance"),
    ("additional_cooling_fan_speed", "Auxiliary part cooling fan"),
    ("chamber_minimal_temperature", "Print chamber temperature"),
    ("chamber_temperature", "Print chamber temperature"),
    ("close_fan_the_first_x_layers", "Cooling for specific layer"),
    ("compatible_printers", "Compatible printers"),
    ("compatible_printers_condition", "Compatible printers"),
    ("compatible_prints", "Compatible process profiles"),
    ("compatible_prints_condition", "Compatible process profiles"),
    ("complete_print_exhaust_fan_speed", "Exhaust fan"),
    ("cool_plate_temp", "Bed temperature"),
    ("cool_plate_temp_initial_layer", "Bed temperature"),
    ("default_filament_colour", "Basic information"),
    ("dont_slow_down_outer_wall", "Part cooling fan"),
    ("during_print_exhaust_fan_speed", "Exhaust fan"),
    ("enable_overhang_bridge_fan", "Part cooling fan"),
    ("enable_pressure_advance", "Flow ratio and Pressure Advance"),
    ("eng_plate_temp", "Bed temperature"),
    ("eng_plate_temp_initial_layer", "Bed temperature"),
    ("fan_cooling_layer_time", "Part cooling fan"),
    ("fan_max_speed", "Part cooling fan"),
    ("fan_min_speed", "Part cooling fan"),
    ("filament_adaptive_volumetric_speed", "Volumetric speed limitation"),
    ("filament_adhesiveness_category", "Basic information"),
    ("filament_change_extrusion_role_gcode", "Change extrusion role G-code"),
    ("filament_change_length", "Basic information"),
    ("filament_colour", "Basic information"),
    ("filament_cooling_final_speed", "Tool change parameters with single extruder MM printers"),
    ("filament_cooling_initial_speed", "Tool change parameters with single extruder MM printers"),
    ("filament_cooling_moves", "Tool change parameters with single extruder MM printers"),
    ("filament_cost", "Basic information"),
    ("filament_density", "Basic information"),
    ("filament_diameter", "Basic information"),
    ("filament_end_gcode", "Filament end G-code"),
    ("filament_flow_ratio", "Flow ratio and Pressure Advance"),
    ("filament_flush_temp", "Multi Filament"),
    ("filament_flush_volumetric_speed", "Multi Filament"),
    ("filament_is_support", "Basic information"),
    ("filament_loading_speed", "Tool change parameters with single extruder MM printers"),
    ("filament_loading_speed_start", "Tool change parameters with single extruder MM printers"),
    ("filament_max_volumetric_speed", "Volumetric speed limitation"),
    ("filament_minimal_purge_on_wipe_tower", "Wipe tower parameters"),
    ("filament_multitool_ramming", "Tool change parameters with multi extruder MM printers"),
    ("filament_multitool_ramming_flow", "Tool change parameters with multi extruder MM printers"),
    ("filament_multitool_ramming_volume", "Tool change parameters with multi extruder MM printers"),
    ("filament_notes", "Notes"),
    ("filament_ramming_parameters", "Tool change parameters with single extruder MM printers"),
    ("filament_shrink", "Basic information"),
    ("filament_shrinkage_compensation_z", "Basic information"),
    ("filament_soluble", "Basic information"),
    ("filament_stamping_distance", "Tool change parameters with single extruder MM printers"),
    ("filament_stamping_loading_speed", "Tool change parameters with single extruder MM printers"),
    ("filament_start_gcode", "Filament start G-code"),
    ("filament_toolchange_delay", "Tool change parameters with single extruder MM printers"),
    ("filament_tower_interface_pre_extrusion_dist", "Wipe tower parameters"),
    ("filament_tower_interface_pre_extrusion_length", "Wipe tower parameters"),
    ("filament_tower_interface_print_temp", "Wipe tower parameters"),
    ("filament_tower_interface_purge_volume", "Wipe tower parameters"),
    ("filament_tower_ironing_area", "Wipe tower parameters"),
    ("filament_type", "Basic information"),
    ("filament_unloading_speed", "Tool change parameters with single extruder MM printers"),
    ("filament_unloading_speed_start", "Tool change parameters with single extruder MM printers"),
    ("filament_vendor", "Basic information"),
    ("full_fan_speed_layer", "Cooling for specific layer"),
    ("hot_plate_temp", "Bed temperature"),
    ("hot_plate_temp_initial_layer", "Bed temperature"),
    ("idle_temperature", "Basic information"),
    ("internal_bridge_fan_speed", "Part cooling fan"),
    ("ironing_fan_speed", "Part cooling fan"),
    ("long_retractions_when_ec", "Multi Filament"),
    ("nozzle_temperature", "Print temperature"),
    ("nozzle_temperature_initial_layer", "Print temperature"),
    ("nozzle_temperature_range_high", "Basic information"),
    ("nozzle_temperature_range_low", "Basic information"),
    ("overhang_fan_speed", "Part cooling fan"),
    ("overhang_fan_threshold", "Part cooling fan"),
    ("pellet_flow_coefficient", "Flow ratio and Pressure Advance"),
    ("pressure_advance", "Flow ratio and Pressure Advance"),
    ("reduce_fan_stop_start_freq", "Part cooling fan"),
    ("retraction_distances_when_ec", "Multi Filament"),
    ("slow_down_for_layer_cooling", "Part cooling fan"),
    ("slow_down_layer_time", "Part cooling fan"),
    ("slow_down_min_speed", "Part cooling fan"),
    ("supertack_plate_temp", "Bed temperature"),
    ("supertack_plate_temp_initial_layer", "Bed temperature"),
    ("support_material_interface_fan_speed", "Part cooling fan"),
    ("temperature_vitrification", "Basic information"),
    ("textured_cool_plate_temp", "Bed temperature"),
    ("textured_cool_plate_temp_initial_layer", "Bed temperature"),
    ("textured_plate_temp", "Bed temperature"),
    ("textured_plate_temp_initial_layer", "Bed temperature"),
];

const FILAMENT_LINES: &[(&str, &str)] = &[
    ("activate_air_filtration_during_print", "During print"),
    ("activate_air_filtration_on_completion", "Complete print"),
    ("chamber_minimal_temperature", "Chamber temperature"),
    ("chamber_temperature", "Chamber temperature"),
    ("complete_print_exhaust_fan_speed", "Complete print"),
    ("cool_plate_temp", "Cool Plate"),
    ("cool_plate_temp_initial_layer", "Cool Plate"),
    ("during_print_exhaust_fan_speed", "During print"),
    ("eng_plate_temp", "Engineering Plate"),
    ("eng_plate_temp_initial_layer", "Engineering Plate"),
    ("fan_cooling_layer_time", "Min fan speed threshold"),
    ("fan_max_speed", "Max fan speed threshold"),
    ("fan_min_speed", "Min fan speed threshold"),
    ("hot_plate_temp", "Smooth PEI Plate / High Temp Plate"),
    ("hot_plate_temp_initial_layer", "Smooth PEI Plate / High Temp Plate"),
    ("nozzle_temperature", "Nozzle"),
    ("nozzle_temperature_initial_layer", "Nozzle"),
    ("nozzle_temperature_range_high", "Recommended nozzle temperature"),
    ("nozzle_temperature_range_low", "Recommended nozzle temperature"),
    ("slow_down_layer_time", "Max fan speed threshold"),
    ("supertack_plate_temp", "Cool Plate (SuperTack)"),
    ("supertack_plate_temp_initial_layer", "Cool Plate (SuperTack)"),
    ("textured_cool_plate_temp", "Textured Cool Plate"),
    ("textured_cool_plate_temp_initial_layer", "Textured Cool Plate"),
    ("textured_plate_temp", "Textured PEI Plate"),
    ("textured_plate_temp_initial_layer", "Textured PEI Plate"),
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

/// The printer-settings category the key appears under in Orca's `TabPrinter`
/// (page for machine-wide keys, optgroup for per-extruder keys), or `None`.
pub fn printer_page_of(key: &str) -> Option<&'static str> {
    PRINTER_PAGES
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| PRINTER_PAGES[i].1)
}

/// The optgroup (sub-section within a page) a machine-wide option appears under,
/// or `None`.
pub fn printer_subgroup_of(key: &str) -> Option<&'static str> {
    PRINTER_SUBGROUPS
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| PRINTER_SUBGROUPS[i].1)
}

/// The filament-settings page the key appears under in Orca's `TabFilament`
/// (Filament, Print temperature, Cooling, …), or `None` for keys not laid out
/// there (metadata, internal). This is the filament editor's visibility signal.
pub fn filament_page_of(key: &str) -> Option<&'static str> {
    FILAMENT_PAGES
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| FILAMENT_PAGES[i].1)
}

/// The optgroup within a filament page (e.g. "Basic information" under the
/// "Filament" page), or `None`.
pub fn filament_subgroup_of(key: &str) -> Option<&'static str> {
    FILAMENT_SUBGROUPS
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| FILAMENT_SUBGROUPS[i].1)
}

/// The label of the multi-option line a filament key sits on (the plate type
/// for bed temps, "Nozzle" for print temps), or `None`. Disambiguates keys
/// whose own label is generic ("Other layers" / "First layer").
pub fn filament_line_of(key: &str) -> Option<&'static str> {
    FILAMENT_LINES
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| FILAMENT_LINES[i].1)
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
        for table in [PRINTER_PAGES, PRINTER_SUBGROUPS, FILAMENT_PAGES, FILAMENT_SUBGROUPS, FILAMENT_LINES] {
            let mut last = "";
            for (key, _) in table {
                assert!(*key > last, "table must be sorted; {key} <= {last}");
                last = key;
            }
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
    fn filament_keys_map_to_their_orca_page() {
        // Page = the add_options_page title; subgroup = the optgroup.
        assert_eq!(filament_page_of("nozzle_temperature"), Some("Filament"));
        assert_eq!(filament_subgroup_of("nozzle_temperature"), Some("Print temperature"));
        assert_eq!(filament_page_of("filament_type"), Some("Filament"));
        assert_eq!(filament_subgroup_of("filament_type"), Some("Basic information"));
        assert_eq!(filament_page_of("fan_max_speed"), Some("Cooling"));
        // Process / printer keys are not in the filament tables.
        assert!(filament_page_of("gcode_flavor").is_none());
        assert!(filament_page_of("layer_height").is_none());
    }

    #[test]
    fn bed_temp_keys_carry_their_plate_line_label() {
        // The plate type is the multi-option line label; the key's own label
        // is just "Other layers" / "First layer".
        assert_eq!(filament_line_of("cool_plate_temp"), Some("Cool Plate"));
        assert_eq!(filament_line_of("textured_plate_temp"), Some("Textured PEI Plate"));
        assert_eq!(filament_line_of("nozzle_temperature"), Some("Nozzle"));
        // A self-labeled single-option line has no line label.
        assert!(filament_line_of("filament_type").is_none());
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
        assert!(filament_page_of("totally_made_up_option").is_none());
        assert!(!is_per_extruder("totally_made_up_option"));
    }
}
