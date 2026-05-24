#!/usr/bin/env python3
"""Convert a BambuStudio device profile triple (machine + process + filament)
into a TOML rule cascade.

Fork of `convert_orca_profile.py` retargeted at the vendored BBS profile
snapshot under `external/BambuStudio-profiles/BBL/` (see that directory's
NOTICE.md for upstream provenance). The differences vs the Orca variant:

  - Reads from `external/BambuStudio-profiles/<vendor>/` instead of
    `external/OrcaSlicer/resources/profiles/<vendor>/`.

  - Resolves BBS's `include:` array. BBS splits G-code macros
    (machine_start_gcode, change_filament_gcode, etc.) into sibling
    template files referenced by name. We pull each template's keys
    into the merged dict at the same precedence as the file that
    declared the include — so a leaf can override a template field,
    but defaults from the template carry through if the leaf doesn't
    mention them.

  - Adds `include` to the META_KEYS set so the directive itself
    doesn't surface in the cascade.

Why a separate script: the Orca converter is referenced by Spike PR-0.5-1's
finding doc; mutating it would break that reproducibility. The BBS chain
is the production source-of-truth path (the Orca one stays as a sanity
mirror for drift detection).

Usage:
    convert_bbs_profile.py \\
        --vendor BBL \\
        --machine "Bambu Lab A1 mini 0.4 nozzle" \\
        --process "0.20mm Standard @BBL A1M" \\
        --filament "Bambu PLA Basic @BBL A1M" \\
        --out profiles/cascades/bambu-a1-mini-default.toml
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
PROFILES_ROOT = REPO_ROOT / "external/BambuStudio-profiles"

# Keys that libslic3r expands per plate type. The cascade carries a
# single logical `bed_temp`; the adapter expands it across all of
# these at apply-time.
PLATE_DIM_KEYS = {
    "hot_plate_temp",
    "hot_plate_temp_initial_layer",
    "cool_plate_temp",
    "cool_plate_temp_initial_layer",
    "eng_plate_temp",
    "eng_plate_temp_initial_layer",
    "textured_plate_temp",
    "textured_plate_temp_initial_layer",
    "textured_cool_plate_temp",
    "textured_cool_plate_temp_initial_layer",
    "supertack_plate_temp",
    "supertack_plate_temp_initial_layer",
    "smooth_plate_temp",
    "smooth_plate_temp_initial_layer",
}

# BBS-to-Orca key renames mirrored verbatim from OrcaSlicer's own
# legacy-handling table (external/OrcaSlicer/src/libslic3r/PrintConfig.cpp,
# the `handle_legacy` chain around lines 7992-8064). When BBS writes
# one of these keys into project_settings.config, Orca's own loader
# would rewrite it before applying — we do the same at convert-time
# so the resulting cascade stays inside our FFI's option vocabulary.
#
# Only pure key→key renames are mirrored here. The handful of BBS
# keys with value-conditional rewrites (`wall_infill_order`,
# `top_one_wall_type`, etc.) land in `DROPPED_KEYS` instead — safer
# to fall back to the libslic3r default than to ship a half-mapped
# value through.
BBS_KEY_RENAMES = {
    "sparse_infill_anchor": "infill_anchor",
    "sparse_infill_anchor_max": "infill_anchor_max",
    "chamber_temperatures": "chamber_temperature",
    "initial_layer_flow_ratio": "bottom_solid_infill_flow_ratio",
    "ironing_direction": "ironing_angle",
    "counterbole_hole_bridging": "counterbore_hole_bridging",
    "prime_tower_extra_rib_length": "wipe_tower_extra_rib_length",
    "prime_tower_rib_width": "wipe_tower_rib_width",
    "prime_tower_fillet_wall": "wipe_tower_fillet_wall",
    "extruder_clearance_max_radius": "extruder_clearance_radius",
    "machine_switch_extruder_time": "machine_tool_change_time",
    "thumbnail_size": "thumbnails",
    # `filament_id` is BBS's singular per-filament identifier
    # ("GFA00"); libslic3r's vector form is `filament_ids`. The Bambu
    # firmware reads `filament_ids` from CONFIG_BLOCK for profile
    # validation — empty value trips an immediate "Print cancelled".
    "filament_id": "filament_ids",
}

# Keys to drop on import. Three buckets:
#
# 1. Orca's own `handle_legacy` ignore set
#    (PrintConfig.cpp:8070-8086) — keys that vanilla libslic3r-as-
#    shipped considers obsolete + silently discards.
# 2. BBS-only firmware extras with no libslic3r counterpart —
#    drying schedules, z-height slowdown curves, scarf-seam params,
#    circle-compensation calibration, AMS-side metadata. libslic3r
#    would accept them only via `--config-overrides`; the printer
#    firmware handles them.
# 3. Value-conditional Orca renames we don't attempt to rewrite
#    (`wall_infill_order`, `top_one_wall_type`, prime_tower_rib_wall,
#    etc.) — dropping is safer than half-mapping.
#
# Sourced from external/OrcaSlicer/src/libslic3r/PrintConfig.cpp +
# a one-time audit of the BBS profile snapshot's set-key surface.
# When BBS adds a new firmware-only key the bootstrap-time validator
# will surface it; add it here and regen.
DROPPED_KEYS = {
    # --- Orca's ignore set ---
    "acceleration", "scale", "rotate", "duplicate", "duplicate_grid",
    "bed_size", "print_center", "g0", "wipe_tower_per_color_wipe",
    "support_sharp_tails", "support_remove_small_overhangs",
    "support_with_sheath", "tree_support_collision_resolution",
    "tree_support_with_infill", "max_volumetric_speed",
    "max_print_speed", "support_closing_radius", "remove_freq_sweep",
    "remove_bed_leveling", "remove_extrusion_calibration",
    "support_transition_line_width", "support_transition_speed",
    "bed_temperature", "bed_temperature_initial_layer",
    "can_switch_nozzle_type", "can_add_auxiliary_fan",
    "extra_flush_volume", "spaghetti_detector", "adaptive_layer_height",
    "z_hop_type", "z_lift_type", "bed_temperature_difference",
    "long_retraction_when_cut", "retraction_distance_when_cut",
    "internal_bridge_support_thickness", "top_area_threshold",
    "reduce_wall_solid_infill", "filament_load_time",
    "filament_unload_time", "smooth_coefficient",
    "overhang_totally_speed", "silent_mode", "overhang_speed_classic",
    "filament_prime_volume",
    # --- Value-conditional Orca renames we don't auto-rewrite ---
    "wall_infill_order",          # → wall_sequence (value-conditional)
    "top_one_wall_type",          # → only_one_wall_top (value-conditional)
    "prime_tower_rib_wall",       # → wipe_tower_wall_type (value-conditional)
    # --- SLA-only meta-key BBS sometimes emits into FFF projects ---
    "printer_technology",
    # --- BBS firmware-only extras (no libslic3r counterpart) ---
    # Bambu z-height velocity ramp
    "slowdown_end_acc", "slowdown_end_height", "slowdown_end_speed",
    "slowdown_start_acc", "slowdown_start_height", "slowdown_start_speed",
    "enable_height_slowdown", "layer_time_smoothing",
    "layer_time_smoothing_threshold",
    # Bambu circle/hole compensation calibration
    "circle_compensation_manual_offset", "circle_compensation_speed",
    "counter_coef_1", "counter_coef_2", "counter_coef_3",
    "counter_limit_max", "counter_limit_min",
    "hole_coef_1", "hole_coef_2", "hole_coef_3",
    "hole_limit_max", "hole_limit_min", "diameter_limit",
    "enable_circle_compensation",
    # AMS-side drying / pre-heating metadata
    "filament_dev_ams_drying_ams_limitations",
    "filament_dev_ams_drying_heat_distortion_temperature",
    "filament_dev_ams_drying_temperature",
    "filament_dev_ams_drying_time",
    "filament_dev_chamber_drying_bed_temperature",
    "filament_dev_chamber_drying_time",
    "filament_dev_drying_cooling_temperature",
    "filament_dev_drying_softening_temperature",
    "enable_pre_heating", "pre_start_fan_time",
    "filament_pre_cooling_temperature",
    "filament_pre_cooling_temperature_nc",
    "filament_preheat_temperature_delta",
    "hotend_cooling_rate", "hotend_heating_rate",
    # Scarf-seam Bambu extension
    "filament_scarf_gap", "filament_scarf_height",
    "filament_scarf_length", "filament_scarf_seam_type",
    "override_filament_scarf_seam_setting", "seam_slope_gap",
    "seam_placement_away_from_overhangs",
    # BBS filament-side per-region overhang speeds (1/4..4/4 + totally)
    "filament_overhang_1_4_speed", "filament_overhang_2_4_speed",
    "filament_overhang_3_4_speed", "filament_overhang_4_4_speed",
    "filament_overhang_totally_speed",
    "filament_enable_overhang_speed",
    "override_process_overhang_speed",
    # Non-chamber/_nc filament variants
    "filament_prime_volume_nc", "filament_ramming_travel_time",
    "filament_ramming_travel_time_nc",
    "filament_ramming_volumetric_speed",
    "filament_ramming_volumetric_speed_nc",
    "filament_retract_length_nc",
    # BBS extruder-clearance/topology extras
    "extruder_clearance_dist_to_rod", "extruder_height_gap",
    "extruder_max_nozzle_count", "filament_extruder_compatibility",
    # Cooling/wall ordering BBS extras
    "cooling_filter_enabled", "cooling_perimeter_transition_distance",
    "cooling_slowdown_logic", "no_slow_down_for_cooling_on_outwalls",
    "support_cooling_filter",
    "monotonic_travel_into_wall", "filament_bridge_speed",
    "vertical_shell_speed", "z_direction_outwall_speed_continuous",
    "travel_short_distance_acceleration",
    "avoid_crossing_wall_includes_support",
    "detect_floating_vertical_shell",
    # BBS toolchanger timing
    "machine_hotend_change_time", "machine_prepare_compensation_time",
    # Misc BBS extras
    "fan_direction", "filament_long_retractions_when_ec",
    "filament_metal_stickiness",
    "filament_retraction_distances_when_ec",
    "filament_velocity_adaptation_factor",
    "group_algo_with_time", "impact_strength_z",
    "infill_instead_top_bottom_surfaces", "infill_rotate_step",
    "locked_skeleton_infill_pattern", "locked_skin_infill_pattern",
    "prime_tower_lift_height", "prime_tower_lift_speed",
    "prime_tower_max_speed", "print_in_clockwise",
    "reduce_infill_retraction_mode",
    "sparse_infill_lattice_angle_1", "sparse_infill_lattice_angle_2",
    "support_ironing_direction", "support_ironing_inset",
    "support_ironing_speed", "enable_support_ironing",
    "top_color_penetration_layers", "bottom_color_penetration_layers",
}

# JSON metadata keys that aren't libslic3r options and should never
# make it into the cascade. Includes BBS-specific keys (`include`,
# `instantiation`) on top of the Orca variant's set.
META_KEYS = {
    "type",
    "name",
    "inherits",
    "include",  # BBS-specific — template-include directive
    "from",
    "setting_id",
    "instantiation",
    "description",
    "compatible_printers",
    "compatible_printers_condition",
    "compatible_prints",
    "compatible_prints_condition",
    # `*_settings_id` look like metadata but the Bambu firmware reads
    # them from the gcode CONFIG_BLOCK to validate the slice against
    # known profiles — empty `filament_settings_id` triggers an
    # immediate "Print cancelled" on the printer. Keep them.
    "upward_compatible_machine",
    "renamed_from",
    "extruder_variant_list",
    "printer_variant",
}


def load_json(path: Path) -> dict:
    with path.open("rb") as f:
        return json.load(f)


def find_by_name(vendor_dir: Path, kind: str, name: str) -> Path:
    """Locate a profile JSON by its `name` field. BBS filenames match
    `name` exactly, but we keep the directory-scan fallback for safety
    (and for the template-include resolution, where the leaf file
    name is sometimes a slight variant)."""
    direct = vendor_dir / kind / f"{name}.json"
    if direct.is_file():
        return direct
    for candidate in (vendor_dir / kind).rglob("*.json"):
        try:
            doc = load_json(candidate)
        except (OSError, json.JSONDecodeError):
            continue
        if doc.get("name") == name:
            return candidate
    sys.exit(f"profile not found: kind={kind} name={name!r} under {vendor_dir}")


def resolve_includes(doc: dict, kind: str, vendor_dir: Path) -> dict:
    """Resolve BBS's `include:` array. Each named template is loaded
    from the same `kind` directory; its keys merge into the doc with
    the doc winning on conflicts. Recursion is supported but unused in
    practice (BBS template files don't chain).

    Returns a fresh dict — does not mutate the caller's `doc`."""
    includes = doc.get("include")
    if not includes:
        return dict(doc)
    merged: dict = {}
    for entry in includes:
        tpl_path = find_by_name(vendor_dir, kind, entry)
        tpl_doc = load_json(tpl_path)
        tpl_resolved = resolve_includes(tpl_doc, kind, vendor_dir)
        for k, v in tpl_resolved.items():
            if k in META_KEYS:
                continue
            merged[k] = v
    # Doc's own keys take precedence over included templates.
    for k, v in doc.items():
        if k == "include":
            continue
        merged[k] = v
    return merged


def flatten_inheritance(start: Path, kind: str, vendor_dir: Path) -> dict:
    """Walk `inherits` from leaf to root + apply each layer's keys
    over the accumulating dict (child wins). `include:` is resolved
    at each layer so a template-provided field can be overridden by
    a deeper inheritance child."""
    chain: list[Path] = []
    current = start
    seen: set[Path] = set()
    while True:
        chain.append(current)
        seen.add(current.resolve())
        doc = load_json(current)
        parent_name = doc.get("inherits")
        if not parent_name:
            break
        parent = find_by_name(vendor_dir, kind, parent_name)
        if parent.resolve() in seen:
            sys.exit(f"inheritance cycle at {parent}")
        current = parent
    merged: dict = {}
    for path in reversed(chain):
        layered = resolve_includes(load_json(path), kind, vendor_dir)
        for k, v in layered.items():
            merged[k] = v
    return merged


def value_to_toml(v) -> str:
    if isinstance(v, list):
        joined = ",".join(str(x) for x in v)
        return _toml_string(joined)
    return _toml_string(str(v))


def _toml_string(s: str) -> str:
    if "\n" in s or '"' in s or "\\" in s:
        return "'''" + s.replace("'''", "''\\''") + "'''"
    return f'"{s}"'


def apply_bbs_filter(
    d: dict,
) -> tuple[dict, list[str], list[tuple[str, str]]]:
    """Filter a raw BBS profile dict through the import rules and
    return `(out, dropped, renamed)`.

    `dropped` is the list of BBS keys we discarded because they're
    in `DROPPED_KEYS` (Orca's own ignore set + BBS-firmware-only
    extras). `renamed` is a list of `(bbs_key, orca_key)` pairs for
    the keys we rewrote via `BBS_KEY_RENAMES`.

    Pure / per-source so callers can accumulate a per-source report
    (machine vs. process vs. filament) for the import surface. META
    + PLATE_DIM keys are filtered silently — those aren't real
    settings and don't warrant a "dropped" entry.

    Designed for reuse: when the runtime project-settings importer
    lands (Phase 7c+), this function (and the constants it consumes)
    is the source-of-truth for what to drop / rename. The constants
    can be exported as JSON for the Rust side to read; the structure
    of this function maps cleanly onto a Rust port.
    """
    out: dict = {}
    dropped: list[str] = []
    renamed: list[tuple[str, str]] = []
    for raw_key, v in d.items():
        if raw_key in META_KEYS or raw_key in PLATE_DIM_KEYS:
            continue
        if raw_key in DROPPED_KEYS:
            dropped.append(raw_key)
            continue
        new_key = BBS_KEY_RENAMES.get(raw_key)
        if new_key is not None:
            renamed.append((raw_key, new_key))
            out[new_key] = v
        else:
            out[raw_key] = v
    return out, dropped, renamed


def write_cascade(out: Path, merged_machine: dict, merged_process: dict,
                  merged_filament: dict, context: dict,
                  source_sha: str) -> tuple[dict, dict]:
    """Render the cascade. Returns `(drops_by_source, renames_by_source)`
    so `main` can print a stdout summary. Each value is keyed by
    `"machine" | "process" | "filament"`."""
    filtered_machine, drops_m, renames_m = apply_bbs_filter(merged_machine)
    filtered_process, drops_p, renames_p = apply_bbs_filter(merged_process)
    filtered_filament, drops_f, renames_f = apply_bbs_filter(merged_filament)

    drops_by_source: dict[str, list[str]] = {
        "machine": drops_m,
        "process": drops_p,
        "filament": drops_f,
    }
    renames_by_source: dict[str, list[tuple[str, str]]] = {
        "machine": renames_m,
        "process": renames_p,
        "filament": renames_f,
    }

    filament_rule_keys = {
        "nozzle_temperature",
        "nozzle_temperature_initial_layer",
        "fan_max_speed",
        "fan_min_speed",
        "fan_cooling_layer_time",
        "slow_down_layer_time",
    }

    default = {**filtered_machine, **filtered_process, **filtered_filament}

    # Synthesize the *_settings_id profile-name keys that BBS auto-
    # derives from profile names at runtime. They aren't stored in
    # the JSON files but the Bambu firmware reads them from the
    # gcode CONFIG_BLOCK to validate the slice — an empty
    # filament_settings_id triggers an immediate "Print cancelled".
    # The converter knows these names from its CLI args so it's the
    # natural place to inject them.
    default["printer_settings_id"] = context["machine"]
    default["filament_settings_id"] = context["filament.name"]
    default["print_settings_id"] = context["process"]
    filament_rule = {k: default.pop(k) for k in list(default) if k in filament_rule_keys}

    bed_temp_value = merged_filament.get("textured_plate_temp", ["55"])
    if isinstance(bed_temp_value, list):
        bed_temp_value = bed_temp_value[0]
    plate_rule = {
        "bed_temp": bed_temp_value,
        "curr_bed_type": "Textured PEI Plate",
    }

    lines: list[str] = []
    lines.append("# Bambu A1 mini — bundled production cascade")
    lines.append("#")
    lines.append("# Generated by scripts/spikes/convert_bbs_profile.py from")
    lines.append("# the vendored BambuStudio profile snapshot under")
    lines.append("# external/BambuStudio-profiles/BBL/. Do not edit by hand.")
    lines.append("#")
    lines.append(f"# BambuStudio upstream SHA: {source_sha}")
    lines.append("#")
    lines.append("# Authored for this resolver context (provenance — the resolver")
    lines.append("# supplies context at runtime, not the cascade):")
    for k, v in context.items():
        lines.append(f"#   {k:<14} = {v!r}")
    lines.extend(_render_import_report(drops_by_source, renames_by_source))
    lines.append("")
    lines.append("# Default rule — specificity 0. Merged machine + process +")
    lines.append("# non-filament-rule filament keys, with plate-dim keys excluded")
    lines.append("# (adapter expands `bed_temp` from the plate rule below).")
    for k in sorted(default):
        lines.append(f"{k} = {value_to_toml(default[k])}")
    lines.append("")
    lines.append("# Filament rule — specificity 1, applies when filament material")
    lines.append('# matches "PLA".')
    lines.append("[[rule]]")
    lines.append('when.filament.type = "PLA"')
    for k in sorted(filament_rule):
        lines.append(f"set.{k} = {value_to_toml(filament_rule[k])}")
    lines.append("")
    lines.append("# Plate rule — specificity 1, applies when plate type matches")
    lines.append('# "Textured PEI". Adapter expands `bed_temp` into per-plate keys.')
    lines.append("[[rule]]")
    lines.append('when.plate.type = "Textured PEI"')
    for k in sorted(plate_rule):
        lines.append(f"set.{k} = {value_to_toml(plate_rule[k])}")
    lines.append("")

    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(lines))

    return drops_by_source, renames_by_source


def _render_import_report(
    drops_by_source: dict[str, list[str]],
    renames_by_source: dict[str, list[tuple[str, str]]],
) -> list[str]:
    """Format the drop / rename report as cascade-header comment
    lines. Stamped into the bundled cascade so the provenance of
    'why isn't BBS key X here?' is self-describing — no need to
    cross-reference the converter script."""
    lines: list[str] = ["#", "# Import report (BBS → cascade):"]

    total_renamed = sum(len(v) for v in renames_by_source.values())
    if total_renamed == 0:
        lines.append("#   Renamed: none")
    else:
        lines.append(
            f"#   Renamed: {total_renamed} key{'s' if total_renamed != 1 else ''}"
        )
        for source in ("machine", "process", "filament"):
            entries = renames_by_source[source]
            for bbs_key, orca_key in sorted(entries):
                lines.append(f"#     {source:<8} {bbs_key} → {orca_key}")

    total_dropped = sum(len(v) for v in drops_by_source.values())
    if total_dropped == 0:
        lines.append("#   Dropped: none")
    else:
        lines.append(
            f"#   Dropped: {total_dropped} key{'s' if total_dropped != 1 else ''} "
            "(BBS firmware-only or in Orca's ignore set)"
        )
        for source in ("machine", "process", "filament"):
            keys = sorted(drops_by_source[source])
            if not keys:
                continue
            lines.append(f"#     from {source}:")
            for k in keys:
                lines.append(f"#       {k}")
    return lines


def _print_import_summary(
    drops_by_source: dict[str, list[str]],
    renames_by_source: dict[str, list[tuple[str, str]]],
) -> None:
    """Compact stdout summary so a regen run surfaces what changed
    without scrolling. Detailed per-key breakdown lives in the
    cascade header."""
    total_renamed = sum(len(v) for v in renames_by_source.values())
    total_dropped = sum(len(v) for v in drops_by_source.values())
    parts = [
        f"renamed {total_renamed} ("
        + ", ".join(
            f"{len(renames_by_source[s])} {s}" for s in ("machine", "process", "filament")
        )
        + ")",
        f"dropped {total_dropped} ("
        + ", ".join(
            f"{len(drops_by_source[s])} {s}" for s in ("machine", "process", "filament")
        )
        + ")",
    ]
    print("import: " + "; ".join(parts))


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--vendor", required=True,
                   help="Vendor directory under external/BambuStudio-profiles/")
    p.add_argument("--machine", required=True, help="Machine profile name")
    p.add_argument("--process", required=True, help="Process profile name")
    p.add_argument("--filament", required=True, help="Filament profile name")
    p.add_argument("--out", required=True, type=Path,
                   help="Output TOML cascade path")
    p.add_argument("--source-sha", default="unknown",
                   help="BambuStudio upstream commit SHA for the vendored snapshot")
    args = p.parse_args()

    vendor_dir = PROFILES_ROOT / args.vendor
    if not vendor_dir.is_dir():
        sys.exit(f"vendor directory missing: {vendor_dir}")

    machine_path = find_by_name(vendor_dir, "machine", args.machine)
    process_path = find_by_name(vendor_dir, "process", args.process)
    filament_path = find_by_name(vendor_dir, "filament", args.filament)

    merged_machine = flatten_inheritance(machine_path, "machine", vendor_dir)
    merged_process = flatten_inheritance(process_path, "process", vendor_dir)
    merged_filament = flatten_inheritance(filament_path, "filament", vendor_dir)

    context = {
        "machine": args.machine,
        "process": args.process,
        "filament.name": args.filament,
        "filament.type": "PLA",
        "plate.type": "Textured PEI",
    }

    drops_by_source, renames_by_source = write_cascade(
        args.out,
        merged_machine,
        merged_process,
        merged_filament,
        context,
        args.source_sha,
    )
    print(f"wrote {args.out}")
    _print_import_summary(drops_by_source, renames_by_source)


if __name__ == "__main__":
    main()
