#!/usr/bin/env python3
"""Import an Orca/BBS filament leaf JSON into our cascade-fragment TOML.

Walks the `inherits` chain across vendor directories (Orca's
filament library frequently inherits from BBL's `fdm_filament_pla` →
`fdm_filament_common` chain, even when the leaf lives under
`OrcaFilamentLibrary/`), flattens root-first, and emits a single
TOML key-value fragment.

Why a dedicated script (vs leaning on `convert_bbs_profile.py`):

  * The existing converter looks parents up only within the same
    `<vendor>/<kind>/` directory as the leaf. That's correct for
    machine and process leaves but breaks on filaments — a leaf in
    `OrcaFilamentLibrary/filament/` that inherits `fdm_filament_pla`
    will never find that parent under the same path, so the chain
    truncates and load-bearing keys like `supertack_plate_temp` from
    `fdm_filament_common.json` never make it into the merged dict.
    libslic3r then falls back to its hardcoded default (35°C for
    supertack), producing the wrong bed temperature without any
    error or warning.

  * This script does cross-vendor parent search — it builds a name
    index across every `filament/` directory under the profiles
    root, with same-vendor preference for tie-breaking, so chains
    that span directory boundaries resolve correctly.

  * Filament leaves don't declare `filament_settings_id`; Orca fills
    it from the preset's `name` at load time. We mirror that here
    so the resulting cascade fragment carries the picker-visible
    display name without needing a runtime backfill.

Usage:
  import_filament_profile.py \\
      --root external/OrcaSlicer/resources/profiles \\
      --leaf "BBL/filament/Generic PLA.json" \\
      --out profiles/vendor/bbl/filament/generic-pla.toml
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


# Envelope keys — describe the file, not the slicer config. Stripped
# during flattening; never emitted.
ENVELOPE_KEYS = frozenset({
    "type",
    "name",
    "inherits",
    "from",
    "setting_id",
    "instantiation",
    "version",
    "renamed_from",
    "filament_settings_id",   # backfilled from leaf `name` (see below)
})


def load_json(path: Path) -> dict:
    return json.loads(path.read_text())


def build_filament_index(root: Path) -> dict[str, list[Path]]:
    """Map `<filename-stem>` → every JSON under any vendor's `filament/`
    directory at any depth. Same name in multiple vendors is allowed
    (we resolve with same-vendor preference at lookup time)."""
    index: dict[str, list[Path]] = {}
    for vendor_dir in root.iterdir():
        if not vendor_dir.is_dir():
            continue
        filament_root = vendor_dir / "filament"
        if not filament_root.is_dir():
            continue
        for p in filament_root.rglob("*.json"):
            index.setdefault(p.stem, []).append(p)
    return index


def resolve_parent(name: str, leaf_dir: Path, index: dict[str, list[Path]]) -> Path:
    candidates = index.get(name)
    if not candidates:
        raise FileNotFoundError(
            f"inherits parent `{name}` not found in any vendor's "
            f"filament/ directory"
        )
    # Prefer a parent that lives in the leaf's own vendor directory.
    # `leaf_dir` is `<root>/<vendor>/filament[/sub]`, so walk up until
    # we find the `filament/` element and compare ancestors.
    leaf_filament_root = leaf_dir
    while leaf_filament_root.name != "filament" and leaf_filament_root.parent != leaf_filament_root:
        leaf_filament_root = leaf_filament_root.parent
    for c in candidates:
        c_filament_root = c.parent
        while c_filament_root.name != "filament" and c_filament_root.parent != c_filament_root:
            c_filament_root = c_filament_root.parent
        if c_filament_root == leaf_filament_root:
            return c
    # Fall back to first candidate (deterministic via Path's __lt__).
    return sorted(candidates)[0]


def flatten_inheritance(leaf: Path, index: dict[str, list[Path]]) -> tuple[dict, list[Path]]:
    """Walk inheritance from `leaf` to root, then merge root-first
    with child-wins. Returns (merged_dict, chain_paths) for the report.
    """
    chain: list[tuple[Path, dict]] = []
    visited: set[Path] = set()
    current = leaf
    while True:
        resolved = current.resolve()
        if resolved in visited:
            raise RuntimeError(f"inherits cycle through {resolved}")
        visited.add(resolved)
        doc = load_json(current)
        chain.append((current, doc))
        parent = doc.get("inherits")
        if not parent:
            break
        current = resolve_parent(parent, current.parent, index)

    merged: dict = {}
    for _, doc in reversed(chain):
        for k, v in doc.items():
            if k in ENVELOPE_KEYS:
                continue
            merged[k] = v
    return merged, [p for p, _ in chain]


def leaf_display_name(leaf: Path) -> str:
    """Use the leaf's `name` field as the picker-visible identity. Falls
    back to the file stem if unset (shouldn't happen on real presets)."""
    doc = load_json(leaf)
    return doc.get("name") or leaf.stem


# ---- TOML emission --------------------------------------------------

def _toml_string(s: str) -> str:
    if "\n" in s or '"' in s or "\\" in s:
        return "'''" + s.replace("'''", "''\\''") + "'''"
    return f'"{s}"'


def value_to_toml(v: Any) -> str:
    """Render a JSON profile value as a TOML right-hand side. Lists
    are comma-joined; libslic3r parses coFloats/coInts/coPercents/
    coBools that way. coStrings keys with embedded commas would need
    a semicolon-joined form, but the BBL PLA chain doesn't hit that
    case in practice — revisit if a future leaf carries
    `gcode_substitutions`-style values.
    """
    if isinstance(v, list):
        return _toml_string(",".join(str(x) for x in v))
    return _toml_string(str(v))


def write_toml(path: Path, kv: dict, *, source: Path, chain: list[Path], root: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    header = [
        f"# {kv.get('filament_settings_id', source.stem)} — filament cascade fragment.",
        f"# Generated by scripts/import_filament_profile.py from:",
        f"#   {source.relative_to(root) if source.is_relative_to(root) else source}",
        f"# Inheritance chain (leaf → root):",
    ]
    for p in chain:
        header.append(f"#   - {p.relative_to(root) if p.is_relative_to(root) else p}")
    header.append("")
    body = [f"{k} = {value_to_toml(kv[k])}" for k in sorted(kv)]
    path.write_text("\n".join(header + body) + "\n")


# ---- Main -----------------------------------------------------------

def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--root", type=Path, required=True,
                    help="profiles tree root (e.g. external/OrcaSlicer/resources/profiles)")
    ap.add_argument("--leaf", required=True,
                    help="filament leaf JSON path, relative to --root "
                         "(e.g. 'BBL/filament/Generic PLA.json')")
    ap.add_argument("--out", type=Path, required=True,
                    help="output TOML path")
    args = ap.parse_args()

    root = args.root.resolve()
    leaf = (root / args.leaf).resolve()
    if not leaf.is_file():
        sys.exit(f"error: leaf not found: {leaf}")

    index = build_filament_index(root)
    merged, chain = flatten_inheritance(leaf, index)

    # Backfill filament_settings_id from leaf name (Orca does this at
    # preset-load time; we hard-stamp it so the picker has a usable
    # display string).
    name = leaf_display_name(leaf)
    merged.setdefault("filament_settings_id", name)
    if not merged.get("filament_settings_id"):
        merged["filament_settings_id"] = name

    print(f"flattened {leaf.relative_to(root)}: {len(merged)} keys, chain depth {len(chain)}")
    for p in chain:
        print(f"  - {p.relative_to(root) if p.is_relative_to(root) else p}")

    write_toml(args.out, merged, source=leaf, chain=chain, root=root)
    print(f"\nwrote {args.out} ({len(merged)} keys)")


if __name__ == "__main__":
    main()
