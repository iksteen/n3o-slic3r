#!/usr/bin/env python3
"""Spike: split a machine family's per-nozzle leaf presets into a common
machine profile + per-nozzle deltas.

For a given vendor + machine family (e.g. BBL + "Bambu Lab A1 mini"),
the upstream OrcaSlicer / BambuStudio profile tree publishes one leaf
JSON per nozzle SKU (0.2 / 0.4 / 0.6 / 0.8 nozzle.json). Each leaf
inherits from a common platform JSON (`fdm_bbl_3dp_001_common.json` →
`fdm_machine_common.json` → ...). After flattening the inheritance
chain you end up with N "full" preset dicts — one per SKU.

Most keys in those dicts are *identical* across SKUs (the printer's
start gcode, printable area, kinematics envelope, etc.). A handful
genuinely differ (the nozzle scalars: diameter, type, volume, retract
geometry). This spike partitions them:

  - Keys identical across all variants  → `<out>/common.json`
    (the true "machine globals" — could power a single shared
    machine profile next to per-nozzle deltas).

  - Keys that differ in at least one variant → `<out>/<sku>.json`
    (carrying just the delta for that variant).

Also prints a structured diff so the human can eyeball which "machine"
settings actually vary across nozzles and which are benign repeats.

Kept separate from the production converter
(`scripts/spikes/convert_bbs_profile.py`) — this is a one-shot
investigation tool, not a release artifact.

Usage:
    machine_nozzle_split.py \\
        --root external/OrcaSlicer/resources/profiles \\
        --vendor BBL \\
        --model "Bambu Lab A1 mini" \\
        --out /tmp/a1mini-split
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


# Preset envelope keys — these describe the file, not the slicer
# config. Strip before merging / diffing.
META_KEYS = {
    "type",
    "name",
    "inherits",
    "from",
    "setting_id",
    "instantiation",
    "filament_settings_id",
    "print_settings_id",
    "printer_settings_id",
    "version",
    "renamed_from",
}

MISSING = object()

# Libslic3r's authoritative per-extruder option list — every key here
# is owned by the extruder dimension regardless of whether its values
# happen to match across nozzle SKUs. Mirrors
# `PrintConfigDef::init_extruder_option_keys()` in
# `external/OrcaSlicer/src/libslic3r/PrintConfig.cpp`. Keep in sync if
# the upstream list grows.
EXTRUDER_KEYS = {
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
}


def load_json(path: Path) -> dict:
    return json.loads(path.read_text())


def find_by_name(machine_dir: Path, name: str) -> Path:
    candidate = machine_dir / f"{name}.json"
    if not candidate.exists():
        raise FileNotFoundError(
            f"inherits parent `{name}` not found under {machine_dir}"
        )
    return candidate


def flatten_inheritance(leaf: Path, machine_dir: Path) -> dict:
    """Walk the `inherits` chain from leaf to root, then merge root-up
    with child-wins. Skips META_KEYS.
    """
    chain: list[dict] = []
    visited: set[Path] = set()
    current = leaf
    while True:
        resolved = current.resolve()
        if resolved in visited:
            raise RuntimeError(f"inherits cycle through {resolved}")
        visited.add(resolved)
        doc = load_json(current)
        chain.append(doc)
        parent_name = doc.get("inherits")
        if not parent_name:
            break
        current = find_by_name(machine_dir, parent_name)

    merged: dict = {}
    # Deepest parent first → leaf last. Each iteration overwrites.
    for doc in reversed(chain):
        for k, v in doc.items():
            if k in META_KEYS:
                continue
            merged[k] = v
    return merged


def discover_variants(machine_dir: Path, model: str) -> list[tuple[str, Path]]:
    """Find every `<model> <sku> nozzle.json` leaf under machine_dir.
    Returns (sku, path) sorted by sku numerically (where parseable),
    else lexicographically. Tolerates parenthesized suffixes (e.g.
    Snapmaker uses `Snapmaker U1 (0.4 nozzle).json`).
    """
    import re

    variants: list[tuple[str, Path]] = []
    for p in sorted(machine_dir.iterdir()):
        if p.suffix != ".json":
            continue
        if not p.stem.startswith(model):
            continue
        suffix = p.stem[len(model):].strip()
        if "nozzle" not in suffix.lower():
            continue
        try:
            doc = load_json(p)
        except json.JSONDecodeError:
            continue
        if doc.get("type") != "machine":
            continue
        # Suffix shapes seen in the wild:
        #   "0.4 nozzle"             (BBL)
        #   "(0.4 nozzle)"           (Snapmaker)
        #   "(0.4+0.6 nozzle)"       (Snapmaker mixed configs)
        # SKU = everything before the literal "nozzle" word, stripped
        # of surrounding punctuation/whitespace.
        m = re.match(r"^[\s()]*(.+?)\s+nozzle\b", suffix, flags=re.IGNORECASE)
        if not m:
            continue
        sku = m.group(1).strip().strip("()")
        if not sku:
            continue
        variants.append((sku, p))

    def sort_key(item: tuple[str, Path]) -> tuple[float, str]:
        sku = item[0]
        try:
            return (float(sku), sku)
        except ValueError:
            return (float("inf"), sku)

    variants.sort(key=sort_key)
    return variants


def split_common_vs_per_variant(
    merged_per_sku: dict[str, dict],
) -> tuple[dict, dict[str, dict], set[str]]:
    """Returns (common, per_variant_delta, differing_keys).
    - common: keys present + equal in every variant.
    - per_variant_delta[sku]: keys where this variant's value differs
      from the consensus (or where some variants don't carry the
      key at all). Includes the actual variant value.
    - differing_keys: set of keys that ended up in any delta dict.
    """
    all_keys: set[str] = set()
    for v in merged_per_sku.values():
        all_keys.update(v.keys())

    common: dict = {}
    per_variant_delta: dict[str, dict] = {sku: {} for sku in merged_per_sku}
    differing_keys: set[str] = set()

    for k in sorted(all_keys):
        first_sku = next(iter(merged_per_sku))
        first_value = merged_per_sku[first_sku].get(k, MISSING)
        all_same = all(
            merged_per_sku[sku].get(k, MISSING) == first_value
            for sku in merged_per_sku
        )
        # libslic3r-declared per-extruder keys always live in the
        # per-variant deltas, even when every variant carries the
        # same value — the composer needs them in the nozzle fragment
        # so it can vector-assemble per extruder.
        if all_same and first_value is not MISSING and k not in EXTRUDER_KEYS:
            common[k] = first_value
            continue
        if all_same:
            # No semantic difference, but per-extruder dimension forces
            # per-nozzle placement. Don't count toward the diff report.
            for sku, doc in merged_per_sku.items():
                val = doc.get(k, MISSING)
                if val is not MISSING:
                    per_variant_delta[sku][k] = val
            continue
        differing_keys.add(k)
        for sku, doc in merged_per_sku.items():
            val = doc.get(k, MISSING)
            if val is not MISSING:
                per_variant_delta[sku][k] = val
    return common, per_variant_delta, differing_keys


def shorten(v: Any, maxlen: int = 100) -> str:
    if v is MISSING:
        return "<absent>"
    s = v if isinstance(v, str) else json.dumps(v, default=str)
    s = s.replace("\n", "\\n")
    if len(s) > maxlen:
        return s[: maxlen - 3] + "..."
    return s


def print_diff_report(
    merged_per_sku: dict[str, dict],
    common: dict,
    per_variant_delta: dict[str, dict],
    differing_keys: set[str],
) -> None:
    skus = list(merged_per_sku.keys())
    print(f"\n=== Diff report for {len(skus)} variants ({', '.join(skus)}) ===")
    print(f"  common keys:      {len(common)}")
    for sku, d in per_variant_delta.items():
        print(f"  {sku} delta keys:  {len(d)}")

    if not differing_keys:
        print("\n  (no keys differ between variants — all merged dicts identical)")
        return

    print(f"\n--- {len(differing_keys)} differing keys ---")
    for k in sorted(differing_keys):
        print(f"\n  {k}:")
        for sku, doc in merged_per_sku.items():
            val = doc.get(k, MISSING)
            print(f"    {sku:>6}: {shorten(val)}")


def write_json_outputs(
    out_dir: Path,
    common: dict,
    per_variant_delta: dict[str, dict],
) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    common_path = out_dir / "common.json"
    common_path.write_text(json.dumps(common, indent=2, sort_keys=True) + "\n")
    print(f"\nwrote {common_path} ({len(common)} keys)")
    for sku, delta in per_variant_delta.items():
        p = out_dir / f"{sku}.json"
        p.write_text(json.dumps(delta, indent=2, sort_keys=True) + "\n")
        print(f"wrote {p} ({len(delta)} keys)")


def _toml_string(s: str) -> str:
    if "\n" in s or '"' in s or "\\" in s:
        return "'''" + s.replace("'''", "''\\''") + "'''"
    return f'"{s}"'


def value_to_toml(v: Any) -> str:
    """Render a JSON profile value as a TOML right-hand side. Lists are
    comma-joined (matches the libslic3r coFloats/coInts/coPercents/
    coStrings serialization for the common case; the existing converter
    has a richer escape mode for coStrings keys with embedded commas,
    but the per-nozzle delta + base-machine keys for the BBL family
    don't hit that case)."""
    if isinstance(v, list):
        joined = ",".join(str(x) for x in v)
        return _toml_string(joined)
    return _toml_string(str(v))


def value_to_toml_scalar(v: Any, *, key: str) -> str:
    """Like `value_to_toml`, but enforces a singular value — per-nozzle
    keys are scoped to one extruder, so 1-element arrays unwrap to
    scalars and multi-element arrays are a structural error (the key
    really belongs in the base machine or is broken-by-design).
    """
    if isinstance(v, list):
        if len(v) == 0:
            # An empty list serializes the same as an empty scalar
            # string in the BBS TOML convention (e.g. `bed_exclude_area
            # = ""`). Keep it singular-shaped.
            return _toml_string("")
        if len(v) > 1:
            raise ValueError(
                f"per-nozzle key `{key}` has {len(v)} elements; per-nozzle "
                f"values must be singular. Drop or move to common.toml. "
                f"Value: {v!r}"
            )
        return _toml_string(str(v[0]))
    return _toml_string(str(v))


def write_toml_outputs(
    toml_root: Path,
    slug: str,
    common: dict,
    per_variant_delta: dict[str, dict],
    *,
    source: str,
) -> None:
    """Emit fragments shaped for the n3o-slic3r vendor profile tree:
      <toml_root>/<slug>.toml             ← base machine
      <toml_root>/<slug>/nozzles/<sku>.toml ← per-nozzle delta
    """
    base_path = toml_root / f"{slug}.toml"
    nozzle_dir = toml_root / slug / "nozzles"
    nozzle_dir.mkdir(parents=True, exist_ok=True)

    base_header = (
        f"# Base machine — keys identical across every nozzle variant.\n"
        f"# Generated by scripts/spikes/machine_nozzle_split.py from:\n"
        f"#   {source}\n"
        f"# Per-nozzle scalars live under `{slug}/nozzles/<sku>.toml`."
    )
    _write_toml(base_path, common, base_header, scalar=False)
    print(f"\nwrote {base_path} ({len(common)} keys)")

    for sku, delta in per_variant_delta.items():
        p = nozzle_dir / f"{sku}.toml"
        nozzle_header = (
            f"# Nozzle SKU `{sku}` — keys that differ from the base machine.\n"
            f"# Per-extruder values; the composer replicates these into\n"
            f"# extruders_count-length vectors at slice time.\n"
            f"# Generated by scripts/spikes/machine_nozzle_split.py."
        )
        _write_toml(p, delta, nozzle_header, scalar=True)
        print(f"wrote {p} ({len(delta)} keys)")


def _write_toml(path: Path, kv: dict, header: str, *, scalar: bool) -> None:
    lines: list[str] = []
    if header:
        lines.extend(header.splitlines())
        lines.append("")
    for k in sorted(kv.keys()):
        rhs = value_to_toml_scalar(kv[k], key=k) if scalar else value_to_toml(kv[k])
        lines.append(f"{k} = {rhs}")
    path.write_text("\n".join(lines) + "\n")


def drop_keys(d: dict, keys: set[str]) -> dict:
    return {k: v for k, v in d.items() if k not in keys}


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", type=Path, required=True,
                    help="profiles tree root (e.g. external/OrcaSlicer/resources/profiles)")
    ap.add_argument("--vendor", required=True,
                    help="vendor directory name under root (e.g. BBL)")
    ap.add_argument("--model", required=True,
                    help='machine model name as it appears in leaf filenames (e.g. "Bambu Lab A1 mini")')
    ap.add_argument("--out", type=Path,
                    help="optional JSON output directory (common.json + <sku>.json)")
    ap.add_argument("--toml-out", type=Path,
                    help="optional TOML output directory shaped for the n3o-slic3r "
                         "vendor profile tree (writes <toml-out>/<slug>.toml + "
                         "<toml-out>/<slug>/nozzles/<sku>.toml)")
    ap.add_argument("--slug",
                    help="machine slug for TOML emission (required with --toml-out)")
    ap.add_argument("--drop", default="",
                    help="comma-separated list of keys to omit from every output. "
                         "Use for libslic3r artefacts that don't belong in the "
                         "n3o-slic3r cascade (e.g. upward_compatible_machine).")
    args = ap.parse_args()

    if args.toml_out and not args.slug:
        ap.error("--toml-out requires --slug")
    if not args.out and not args.toml_out:
        ap.error("specify at least one of --out / --toml-out")

    machine_dir = args.root / args.vendor / "machine"
    if not machine_dir.is_dir():
        print(f"error: machine dir not found: {machine_dir}", file=sys.stderr)
        sys.exit(1)

    variants = discover_variants(machine_dir, args.model)
    if not variants:
        print(f"error: no `{args.model} <sku> nozzle.json` leaves under {machine_dir}", file=sys.stderr)
        sys.exit(1)

    print(f"discovered {len(variants)} variant(s) for `{args.model}`:")
    for sku, p in variants:
        print(f"  {sku}: {p.relative_to(args.root)}")

    merged_per_sku: dict[str, dict] = {}
    for sku, leaf in variants:
        merged_per_sku[sku] = flatten_inheritance(leaf, machine_dir)
        print(f"  {sku}: flattened to {len(merged_per_sku[sku])} keys "
              f"(chain depth {_chain_depth(leaf, machine_dir)})")

    common, per_variant_delta, differing_keys = split_common_vs_per_variant(merged_per_sku)

    drop_set = {k.strip() for k in args.drop.split(",") if k.strip()}
    if drop_set:
        before = (len(common), {sku: len(d) for sku, d in per_variant_delta.items()})
        common = drop_keys(common, drop_set)
        per_variant_delta = {
            sku: drop_keys(d, drop_set) for sku, d in per_variant_delta.items()
        }
        print(f"\nDropping {len(drop_set)} key(s): {sorted(drop_set)}")
        print(f"  common: {before[0]} → {len(common)} keys")
        for sku, d in per_variant_delta.items():
            print(f"  {sku} delta: {before[1][sku]} → {len(d)} keys")

    print_diff_report(merged_per_sku, common, per_variant_delta, differing_keys)

    if args.out:
        write_json_outputs(args.out, common, per_variant_delta)
    if args.toml_out:
        source = f"{args.root}/{args.vendor}/machine/{args.model} <sku> nozzle.json"
        write_toml_outputs(
            args.toml_out, args.slug, common, per_variant_delta, source=source,
        )


def _chain_depth(leaf: Path, machine_dir: Path) -> int:
    depth = 1
    current = leaf
    while True:
        doc = load_json(current)
        parent = doc.get("inherits")
        if not parent:
            return depth
        current = find_by_name(machine_dir, parent)
        depth += 1


if __name__ == "__main__":
    main()
