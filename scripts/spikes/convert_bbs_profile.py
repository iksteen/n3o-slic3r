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
    "printer_settings_id",
    "filament_settings_id",
    "print_settings_id",
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


def write_cascade(out: Path, merged_machine: dict, merged_process: dict,
                  merged_filament: dict, context: dict, source_sha: str) -> None:
    def filter_keys(d: dict) -> dict:
        return {
            k: v for k, v in d.items()
            if k not in META_KEYS and k not in PLATE_DIM_KEYS
        }

    filament_rule_keys = {
        "nozzle_temperature",
        "nozzle_temperature_initial_layer",
        "fan_max_speed",
        "fan_min_speed",
        "fan_cooling_layer_time",
        "slow_down_layer_time",
    }

    default = filter_keys({**merged_machine, **merged_process, **merged_filament})
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

    write_cascade(args.out, merged_machine, merged_process, merged_filament,
                  context, args.source_sha)
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
