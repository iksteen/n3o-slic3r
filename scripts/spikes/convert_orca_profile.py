#!/usr/bin/env python3
"""Convert an OrcaSlicer device profile triple (machine + process + filament)
into a TOML rule cascade for spike PR-0.5-1.

The output cascade follows the shape documented in docs/profiles.md:
a default rule with the bulk of the merged settings, a filament rule
for the filament-origin keys, and a plate rule with `bed_temp` as a
logical key (the spike's stub adapter is responsible for expanding it
into libslic3r's per-plate-type vector keys).

Spike code; not optimized. Idempotent for the same inputs.

Usage:
    convert_orca_profile.py \\
        --vendor BBL \\
        --machine "Bambu Lab A1 mini 0.4 nozzle" \\
        --process "0.20mm Standard @BBL A1M" \\
        --filament "Bambu PLA Basic @BBL A1M" \\
        --out examples/cascades/bambu-a1-mini-spike1.toml
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
PROFILES_ROOT = REPO_ROOT / "external/OrcaSlicer/resources/profiles"

# Keys that libslic3r expands per plate type. The spike's cascade carries
# a single logical `bed_temp` key in the plate rule; the adapter expands
# it across all of these at apply-time.
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

# OrcaSlicer JSON metadata keys that aren't libslic3r options and should
# never make it into the cascade.
META_KEYS = {
    "type",
    "name",
    "inherits",
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
    """Locate a profile JSON by its `name` field. OrcaSlicer's filenames
    usually match `name` but not always (some omit nozzle suffixes); fall
    back to a directory scan."""
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


def flatten_inheritance(start: Path, kind: str, vendor_dir: Path) -> dict:
    """Walk `inherits` from leaf to root, then apply each layer's keys
    over the accumulating dict. Child wins."""
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
        for k, v in load_json(path).items():
            merged[k] = v
    return merged


def value_to_toml(v) -> str:
    """Serialize a JSON value as a TOML scalar. OrcaSlicer values are
    always either string or list-of-string; we keep them as strings to
    match the `Config::set(key, &str)` contract the libslic3r FFI
    expects (libslic3r parses the comma-separated form for vectors)."""
    if isinstance(v, list):
        joined = ",".join(str(x) for x in v)
        return _toml_string(joined)
    return _toml_string(str(v))


def _toml_string(s: str) -> str:
    if "\n" in s or '"' in s or "\\" in s:
        return "'''" + s.replace("'''", "''\\''") + "'''"
    return f'"{s}"'


def write_cascade(out: Path, merged_machine: dict, merged_process: dict,
                  merged_filament: dict, context: dict) -> None:
    """Emit the TOML cascade. Three rule blocks:

    1. Default rule (specificity 0): machine + process keys + filament
       keys that aren't filament-context-specific. This is the bulk.
    2. Filament rule (specificity 1, when.filament.type = ...): filament
       keys whose semantics are tied to the filament material.
    3. Plate rule (specificity 1, when.plate.type = ...): a single
       logical `bed_temp` plus `curr_bed_type` selector. The adapter
       expands `bed_temp` into all `*_plate_temp*` libslic3r keys.

    Plate-dimensional keys (hot_plate_temp, etc.) are *excluded* from the
    cascade entirely; the adapter writes them based on the resolved
    `bed_temp`."""

    def filter_keys(d: dict) -> dict:
        return {
            k: v for k, v in d.items()
            if k not in META_KEYS and k not in PLATE_DIM_KEYS
        }

    # Filament-origin keys that don't make sense as defaults — they get
    # an `when.filament.type = "PLA"` predicate. Everything else from
    # the filament profile (filament_density, filament_cost, etc.)
    # could equally well live there, but for the spike we just pull
    # the temperature/fan settings into the filament rule to exercise
    # the cascade's specificity ladder.
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

    # Pull the textured-plate temp out of the merged filament dict for
    # the plate rule's logical bed_temp value. `curr_bed_type` is the
    # libslic3r selector key.
    bed_temp_value = merged_filament.get("textured_plate_temp", ["55"])
    if isinstance(bed_temp_value, list):
        bed_temp_value = bed_temp_value[0]
    plate_rule = {
        "bed_temp": bed_temp_value,
        "curr_bed_type": "Textured PEI Plate",
    }

    lines: list[str] = []
    lines.append("# Spike PR-0.5-1 — A1 mini + PLA + Textured PEI cascade")
    lines.append("# Generated by scripts/spikes/convert_orca_profile.py from")
    lines.append("# OrcaSlicer profiles. Do not edit by hand; rerun the converter.")
    lines.append("#")
    lines.append("# Authored for this resolver context (provenance only — the")
    lines.append("# resolver supplies the context at runtime, not the cascade):")
    for k, v in context.items():
        lines.append(f"#   {k:<14} = {v!r}")
    lines.append("")
    lines.append("# Default rule — specificity 0. The merged machine + process +")
    lines.append("# non-filament-rule filament keys, with plate-dim keys excluded.")
    lines.append("# Emitted as top-level keys (the recommended form for the")
    lines.append("# unconditional default — see docs/profiles.md). Must appear")
    lines.append("# before any [[rule]] header.")
    for k in sorted(default):
        lines.append(f"{k} = {value_to_toml(default[k])}")
    lines.append("")
    lines.append("# Filament rule — specificity 1, applies when context's filament")
    lines.append('# material matches "PLA". Sets the temperature and fan curves')
    lines.append("# that depend on the filament's material.")
    lines.append("[[rule]]")
    lines.append('when.filament.type = "PLA"')
    for k in sorted(filament_rule):
        lines.append(f"set.{k} = {value_to_toml(filament_rule[k])}")
    lines.append("")
    lines.append("# Plate rule — specificity 1, applies when context's plate type")
    lines.append('# is "Textured PEI". Carries a logical `bed_temp` which the')
    lines.append("# stub adapter expands into libslic3r's per-plate-type vector")
    lines.append("# keys (hot_plate_temp, cool_plate_temp, ...).")
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
                   help="Vendor directory under external/OrcaSlicer/resources/profiles/")
    p.add_argument("--machine", required=True, help="Machine profile name")
    p.add_argument("--process", required=True, help="Process profile name")
    p.add_argument("--filament", required=True, help="Filament profile name")
    p.add_argument("--out", required=True, type=Path,
                   help="Output TOML cascade path")
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
                  context)
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
