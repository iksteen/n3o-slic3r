#!/usr/bin/env python3
"""Import a machine family's per-nozzle leaf JSONs into n3o-slic3r's
base-machine + per-nozzle profile layout.

n3o-slic3r treats each printer as ONE base machine plus N nozzle
profiles a user picks per toolhead. Upstream Orca / BBS profiles
instead ship one full leaf preset per (printer × nozzle SKU) combo —
sometimes with mixed-nozzle "0.4+0.6" leaves stacked on top — and
encode any per-toolhead choice inside the preset's per-extruder
vectors. This script collapses that organic mess into our layout.

Algorithm:
  1. Discover every leaf machine JSON matching `<model>* nozzle*.json`
     under `<root>/<vendor>/machine/`. Handles both BBL's
     `<model> 0.4 nozzle.json` and Snapmaker's
     `<model> (0.4 nozzle).json` shapes.
  2. Flatten each leaf's `inherits` chain into a single dict.
  3. Partition the flattened dict by libslic3r's authoritative
     per-extruder key set (mirrors
     `PrintConfigDef::init_extruder_option_keys`):
       - keys IN  EXTRUDER_KEYS → per-nozzle profile
       - keys NOT IN  EXTRUDER_KEYS → base machine profile
  4. Conflict check: a machine-side key may be inherited silently by
     any number of variants, but if more than one *leaf* declares it
     explicitly and the declared values differ, the source data is
     inconsistent and we abort with a diff.
  5. Emit:
       <toml-out>/<slug>.toml             ← base machine
       <toml-out>/<slug>/nozzles/<sku>.toml   ← per-nozzle (scalars)

The conflict check is the key safety property: it lets us trust the
canonical leaf's machine-wide overrides while accepting silently-
inherited divergence as the "firmware era mismatch" cost. If a vendor
ever ships two leaves that both *declare* conflicting machine-wide
settings, the script refuses to import — the human has to pick.

Usage:
  import_machine_profile.py \\
      --root external/OrcaSlicer/resources/profiles \\
      --vendor BBL \\
      --model "Bambu Lab A1 mini" \\
      --slug bambu-lab-a1-mini \\
      --toml-out profiles/vendor/bbl/printer
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


# Envelope metadata — describes the file, not the slicer config. Never
# emitted.
ENVELOPE_KEYS = frozenset({
    "type",
    "name",
    "inherits",
    "from",
    "setting_id",
    "instantiation",
    "version",
    "renamed_from",
    "filament_settings_id",
    "print_settings_id",
    "printer_settings_id",
})

# Picker / UX defaults declared in the model JSON (`type: "machine_model"`)
# that are real machine-side config we want in the output. The model JSON
# is a sibling of the per-nozzle leaves rather than a parent in the
# `inherits` chain, so the leaf walker never reaches it — we merge these
# keys explicitly. Keys NOT in this set get dropped (envelope-y picker
# fields like `bed_model`, `bed_texture`, `image_bed_type`, `url`,
# `family`, `model_id`, `machine_tech` don't belong in the cascade).
MODEL_JSON_KEYS = frozenset({
    "default_bed_type",
    "not_support_bed_type",
    "default_materials",
})

# Catalog/picker metadata that travels in machine leaves but isn't
# slicer config and doesn't belong in our cascade. Dropped by default.
#
# `printer_model` is NOT in this set — it looks like metadata but
# libslic3r's FFI uses it to gate vendor-specific validations (e.g.
# `is_BBL_printer()` switches off Marlin-flavor checks). Dropping it
# trips the "relative extruder addressing requires G92 E0 in
# layer_gcode" validate.
DEFAULT_DROPPED_META = frozenset({
    "default_print_profile",       # picker UX hint, broken for mixed-nozzle setups
    "upward_compatible_machine",   # libslic3r cross-printer compat artefact
})

# Keys that don't belong in either dimension but DO need to ride along
# with the nozzle profile (e.g. SKU identity tag).
NOZZLE_IDENTITY_KEYS = frozenset({
    "printer_variant",
})

# libslic3r's authoritative per-extruder option list — every key here
# is owned by the extruder dimension regardless of whether its values
# happen to match across nozzle SKUs. Mirrors
# `PrintConfigDef::init_extruder_option_keys()` in
# `external/OrcaSlicer/src/libslic3r/PrintConfig.cpp`. Keep in sync if
# the upstream list grows.
EXTRUDER_KEYS = frozenset({
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
})


# ---- Leaf discovery + inheritance flattening -----------------------


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
    """Walk the `inherits` chain leaf-to-root and merge root-first so
    child values overwrite parents."""
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
        parent = doc.get("inherits")
        if not parent:
            break
        current = find_by_name(machine_dir, parent)

    merged: dict = {}
    for doc in reversed(chain):  # deepest first; leaf last
        for k, v in doc.items():
            if k in ENVELOPE_KEYS:
                continue
            merged[k] = v
    return merged


def discover_variants(machine_dir: Path, model: str) -> list[tuple[str, Path]]:
    """Find every `<model> <sku> nozzle.json` leaf under `machine_dir`.
    Handles both unparenthesized (BBL) and parenthesized (Snapmaker)
    SKU suffixes. Returns `(sku, path)` sorted numerically by SKU when
    parseable, else lexicographically.
    """
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
        # Suffix shapes:
        #   "0.4 nozzle"                (BBL)
        #   "(0.4 nozzle)"              (Snapmaker)
        #   "(0.4+0.6 nozzle)"          (Snapmaker mixed)
        # SKU = everything between the leading punctuation/space and
        # the literal "nozzle" word.
        m = re.match(r"^[\s()]*(.+?)\s+nozzle\b", suffix, flags=re.IGNORECASE)
        if not m:
            continue
        sku = m.group(1).strip().strip("()")
        if not sku:
            continue
        variants.append((sku, p))

    def sort_key(item: tuple[str, Path]) -> tuple[float, str]:
        try:
            return (float(item[0]), item[0])
        except ValueError:
            return (float("inf"), item[0])

    variants.sort(key=sort_key)
    return variants


# ---- Partition + conflict detection --------------------------------


def leaf_declarations(leaf: Path, drop_keys: frozenset[str]) -> dict:
    """Return the leaf JSON's own declarations (NOT the flattened
    chain). Drops `ENVELOPE_KEYS` and the user-supplied drop set so
    the conflict check ignores keys we'd discard from output anyway.
    """
    doc = load_json(leaf)
    return {
        k: v
        for k, v in doc.items()
        if k not in ENVELOPE_KEYS and k not in drop_keys
    }


def detect_machine_conflicts(
    leaves: dict[str, dict],
) -> list[tuple[str, dict[str, Any]]]:
    """For each machine-side key declared by more than one leaf,
    return the per-SKU declarations IF they don't all agree. Returned
    as `[(key, {sku: value, ...}), ...]` so the caller can render a
    diff.
    """
    by_key: dict[str, dict[str, Any]] = {}
    for sku, decls in leaves.items():
        for k, v in decls.items():
            if k in EXTRUDER_KEYS or k in NOZZLE_IDENTITY_KEYS:
                continue
            by_key.setdefault(k, {})[sku] = v

    conflicts: list[tuple[str, dict[str, Any]]] = []
    for k, decls in by_key.items():
        if len(decls) <= 1:
            continue
        # Compare via canonical-JSON so nested lists/dicts compare
        # structurally.
        signatures = {sku: json.dumps(v, sort_keys=True) for sku, v in decls.items()}
        if len(set(signatures.values())) > 1:
            conflicts.append((k, decls))
    return conflicts


def render_conflict_report(conflicts: list[tuple[str, dict[str, Any]]]) -> str:
    lines: list[str] = [
        f"error: {len(conflicts)} machine-side key(s) declared inconsistently across variants:",
        "",
    ]
    for key, decls in conflicts:
        lines.append(f"  {key}:")
        for sku, value in decls.items():
            shown = json.dumps(value, default=str)
            if len(shown) > 100:
                shown = shown[:97] + "..."
            lines.append(f"    {sku!r:>10}: {shown}")
        lines.append("")
    lines.append(
        "Resolve by either:\n"
        "  - patching the upstream profile so all leaves agree, or\n"
        "  - dropping the offending leaf via --skip <sku>, or\n"
        "  - explicitly dropping the conflicting key(s) via --drop"
    )
    return "\n".join(lines)


def build_base_machine(
    flat: dict[str, dict],
    leaf: dict[str, dict],
) -> dict:
    """Construct the base machine config from per-variant flattened
    dicts + their leaf declarations. Caller has already run the
    conflict check, so declared values are guaranteed unique per key.

    For each machine-side key seen anywhere in any variant's flat:
      - If any leaf declares it explicitly, use the declared value
        (canonical / latest firmware).
      - Else use the value from any variant's flattened chain (they
        all inherit the same parent path for this key, so any
        variant works).
    """
    machine: dict = {}
    seen: set[str] = set()
    for d in flat.values():
        for k in d:
            if k in EXTRUDER_KEYS or k in NOZZLE_IDENTITY_KEYS:
                continue
            seen.add(k)

    for k in seen:
        declared = [decls[k] for decls in leaf.values() if k in decls]
        if declared:
            machine[k] = declared[0]
            continue
        for d in flat.values():
            if k in d:
                machine[k] = d[k]
                break
    return machine


def build_nozzle_profile(flat_variant: dict) -> dict:
    """Per-nozzle: every per-extruder key from the flattened dict, plus
    the SKU identity tag(s)."""
    out: dict = {}
    for k, v in flat_variant.items():
        if k in EXTRUDER_KEYS or k in NOZZLE_IDENTITY_KEYS:
            out[k] = v
    return out


# ---- TOML emission -------------------------------------------------


def _toml_string(s: str) -> str:
    if "\n" in s or '"' in s or "\\" in s:
        return "'''" + s.replace("'''", "''\\''") + "'''"
    return f'"{s}"'


def _value_to_toml_machine(v: Any) -> str:
    """Machine-side TOML emission. Lists are comma-joined (matches the
    BBS coFloats/coInts/coPercents serialization)."""
    if isinstance(v, list):
        return _toml_string(",".join(str(x) for x in v))
    return _toml_string(str(v))


def _value_to_toml_nozzle(v: Any, *, key: str) -> str:
    """Per-nozzle TOML emission. Forces singular values.

    Upstream leaves wrap per-extruder values in length-N vectors (one
    entry per toolhead in the original preset). For a pure
    single-SKU variant every entry is identical, so we collapse to
    the scalar. Heterogeneous vectors mean the leaf encodes a
    multi-nozzle topology (e.g. the 0.4+0.6 mixed config) which
    doesn't belong in a single-SKU nozzle profile — that's the
    structural-error case.
    """
    if isinstance(v, list):
        if len(v) == 0:
            # Empty list serializes the same as an empty scalar string
            # in BBS convention (e.g. `extruder_printable_height = ""`).
            return _toml_string("")
        unique = {json.dumps(x, sort_keys=True): x for x in v}
        if len(unique) > 1:
            raise ValueError(
                f"nozzle key `{key}` has heterogeneous values "
                f"({len(unique)} distinct in a length-{len(v)} vector); "
                f"this leaf encodes a mixed-nozzle topology, not a "
                f"single-SKU nozzle profile. Skip this variant or drop "
                f"the key. Value: {v!r}"
            )
        return _toml_string(str(v[0]))
    return _toml_string(str(v))


def write_base_machine(path: Path, kv: dict, *, source_root: str, slug: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    header = [
        "# Base machine — keys shared across every nozzle variant.",
        f"# Generated by scripts/import_machine_profile.py from:",
        f"#   {source_root}",
        f"# Per-nozzle scalars live under `{slug}/nozzles/<sku>.toml`.",
        "",
    ]
    body = [f"{k} = {_value_to_toml_machine(kv[k])}" for k in sorted(kv)]
    path.write_text("\n".join(header + body) + "\n")


def write_nozzle_profile(path: Path, kv: dict, *, sku: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    header = [
        f"# Nozzle SKU `{sku}` — per-extruder values.",
        "# The composer replicates these into extruders_count-length",
        "# vectors at slice time (one nozzle.toml loaded per extruder).",
        "# Generated by scripts/import_machine_profile.py.",
        "",
    ]
    body = [f"{k} = {_value_to_toml_nozzle(kv[k], key=k)}" for k in sorted(kv)]
    path.write_text("\n".join(header + body) + "\n")


# ---- Main ----------------------------------------------------------


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--root", type=Path, required=True,
                    help="profiles tree root (e.g. external/OrcaSlicer/resources/profiles)")
    ap.add_argument("--vendor", required=True,
                    help="vendor directory name under root (e.g. BBL, Snapmaker)")
    ap.add_argument("--model", required=True,
                    help='machine model name as it appears in leaf filenames')
    ap.add_argument("--slug", required=True,
                    help="machine slug (target filename + dir name in our profile tree)")
    ap.add_argument("--toml-out", type=Path, required=True,
                    help="output directory shaped for the n3o-slic3r vendor profile tree")
    ap.add_argument("--drop", default="",
                    help="comma-separated extra keys to omit from every output, "
                         "on top of the defaults: " + ", ".join(sorted(DEFAULT_DROPPED_META)))
    ap.add_argument("--skip", default="",
                    help="comma-separated SKUs to ignore (e.g. mixed-nozzle 0.4+0.6 leaves "
                         "are typically not real machine variants)")
    args = ap.parse_args()

    machine_dir = args.root / args.vendor / "machine"
    if not machine_dir.is_dir():
        sys.exit(f"error: machine dir not found: {machine_dir}")

    drop_keys = frozenset(
        list(DEFAULT_DROPPED_META)
        + [k.strip() for k in args.drop.split(",") if k.strip()]
    )
    skip_skus = {s.strip() for s in args.skip.split(",") if s.strip()}

    variants = discover_variants(machine_dir, args.model)
    variants = [(sku, p) for sku, p in variants if sku not in skip_skus]
    if not variants:
        sys.exit(f"error: no `{args.model} <sku> nozzle.json` leaves under {machine_dir}")

    print(f"importing {len(variants)} variant(s) of `{args.model}`:")
    for sku, p in variants:
        print(f"  {sku}: {p.relative_to(args.root)}")

    # Flatten chains + capture leaf declarations.
    flat: dict[str, dict] = {}
    leaf: dict[str, dict] = {}
    for sku, path in variants:
        flat[sku] = {
            k: v for k, v in flatten_inheritance(path, machine_dir).items()
            if k not in drop_keys
        }
        leaf[sku] = leaf_declarations(path, drop_keys)

    # Conflict check against the leaf-declaration set.
    conflicts = detect_machine_conflicts(leaf)
    if conflicts:
        print(render_conflict_report(conflicts), file=sys.stderr)
        sys.exit(2)

    # Build outputs.
    base_machine = build_base_machine(flat, leaf)

    # Merge picker defaults from the model JSON (`<model>.json`,
    # type "machine_model"). These are sibling-not-parent fields the
    # leaf walker can't reach. Only merge keys the leaf chain didn't
    # already declare — the leaf is authoritative when both speak.
    model_json = machine_dir / f"{args.model}.json"
    if model_json.exists():
        try:
            mdoc = load_json(model_json)
        except json.JSONDecodeError as e:
            print(f"warning: couldn't parse {model_json}: {e}", file=sys.stderr)
        else:
            if mdoc.get("type") == "machine_model":
                added = 0
                for k in MODEL_JSON_KEYS:
                    if k in mdoc and k not in base_machine:
                        base_machine[k] = mdoc[k]
                        added += 1
                if added:
                    print(
                        f"\nmerged {added} picker-default(s) from "
                        f"{model_json.relative_to(args.root)}"
                    )

    nozzle_profiles = {sku: build_nozzle_profile(flat[sku]) for sku in flat}

    # Emit TOML.
    source_root = f"{args.root}/{args.vendor}/machine/"
    base_path = args.toml_out / f"{args.slug}.toml"
    write_base_machine(base_path, base_machine, source_root=source_root, slug=args.slug)
    print(f"\nwrote {base_path} ({len(base_machine)} keys)")

    nozzle_dir = args.toml_out / args.slug / "nozzles"
    for sku, kv in nozzle_profiles.items():
        p = nozzle_dir / f"{sku}.toml"
        write_nozzle_profile(p, kv, sku=sku)
        print(f"wrote {p} ({len(kv)} keys)")


if __name__ == "__main__":
    main()
