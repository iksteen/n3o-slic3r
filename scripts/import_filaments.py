#!/usr/bin/env python3
"""Consolidate upstream Orca/BBS filament leaves into our cascade
fragments. Drives both per-brand consolidation (Bambu Lab, Snapmaker,
…) and cross-vendor Generic consolidation; the `--brand` arg selects
the mode.

Input shape — Orca/BBS per-(filament × printer × maybe-nozzle) leaves:

    BBL/filament/Bambu PLA Basic @BBL A1.json
    BBL/filament/Bambu PLA Basic @BBL X1C 0.4 nozzle.json
    BBL/filament/Generic PLA @BBL X1C.json
    Dremel/filament/Dremel Generic PLA @3D20 all.json
    OrcaArena/filament/OrcaArena Generic PLA Silk @base.json
    CONSTRUCT3D/filament/C1 Generic PLA.json
    Snapmaker/filament/Snapmaker PLA @U1.json
    ...

Output shape — one rule-based cascade fragment per logical filament:

    resources/profiles/bbl/filament/bambu-pla-basic.toml
    resources/profiles/generic/filament/generic-pla.toml
    resources/profiles/generic/filament/generic-pla-silk.toml
    resources/profiles/snapmaker/filament/snapmaker-pla.toml
    ...

Bucket-extraction strategy depends on `--brand`:

  * `--brand Generic` (cross-vendor): vendor prefix stripped, names
    pivot the `Creality HF Generic PLA` quirk so it folds with the
    canonical `Generic <Material> HF` form. `Dremel Generic PLA` and
    `Generic PLA` both end up in `generic-pla.toml`.

  * `--brand "Bambu Lab"`, `--brand Snapmaker`, etc.: the full leaf
    name (minus `@<printer>` and trailing nozzle) is the logical
    name; each branded product is its own bucket.

Algorithm:

  1. Discover every `*.json` leaf under any `<vendor>/filament/`,
     filter by `filament_vendor == [<brand>]`, bucket by slugified
     logical name. Fold nozzle-suffixed leaves into their no-nozzle
     parent (0.4-nozzle fallback when only nozzle-keyed variants
     exist) — nozzle is a separate cascade dim that
     `compatible_printers` doesn't carry, so retaining all variants
     pollutes deltas.

  2. For each bucket, collect per-(printer_model, key) values from
     every leaf, expanding `compatible_printers` into per-printer
     entries (cleaning vendor-style nozzle suffixes off each entry).
     Conflicts (same printer × key from two leaves): non-nozzle
     beats nozzle, smaller `compatible_printers` (more specific)
     beats larger, then BBL > alphabetical. Genuine cross-vendor
     authoring conflicts get logged.

  3. Baseline = most-common value per key across the bucket's
     printers. Per-printer rules emit keys where the printer's value
     differs from baseline.

  4. Drop deny-list keys entirely (see `DENY_KEYS`): vestigial fields
     libslic3r doesn't read, pure UI metadata, and printer-binding
     leaks like `filament_extruder_variant` (pending the per-extruder
     cascade dim described in `docs/profiles.md`).

  5. Normalize values: `"12,12"` → `"12"` when all per-extruder
     elements agree, so single- and multi-extruder leaves cluster.

  6. Group printers by identical full delta-set and emit one
     `[[rule]] when.printer.model = [...]` per group (the cascade
     resolver evaluates the array as OR membership).

Usage:

    import_filaments.py \\
        --root external/OrcaSlicer/resources/profiles \\
        --out resources/profiles/bbl/filament \\
        --brand "Bambu Lab"

Existing files in the output dir are NOT removed automatically.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

from _atomic_io import atomic_write_text
from _profile_typos import fold_typo_keys


ENVELOPE_KEYS = frozenset({
    "type",
    "name",
    "inherits",
    "from",
    "setting_id",
    "instantiation",
    "version",
    "renamed_from",
})

# Keys that exist in upstream filament leaves but don't belong in a
# generic filament cascade fragment. Two categories:
#
#   - **Vestigial** (zero refs in libslic3r): historical fields that
#     no slicer code reads. `bed_type` is the headline example — the
#     code that read it (`OptionsGroup.cpp:697`) is `#if 0`'d; the
#     surviving `config_value("bed_type", ...)` handler returns a
#     hardcoded `btPC` fallback. The actual bed-temp flow uses
#     `curr_bed_type` (project scope, set by the Plater UI) +
#     `get_bed_temp_key()` to look up per-plate-type keys
#     (`cool_plate_temp`, `eng_plate_temp`, …). The plural
#     `chamber_temperatures` is similarly unread (only singular
#     `chamber_temperature` is). The Bambu first-layer calibration
#     constants (`circle_compensation_*`, `counter_*`) are JSON-only.
#
#   - **Pure UI metadata**: color-picker defaults, free-text notes,
#     UI-gating bit-masks, empty `compatible_*` stub fields. None
#     affect the slice.
#
#   - **Printer-binding leak**: `filament_extruder_variant` is read
#     (Print.cpp:2974) to remap filament options to physical
#     extruders, but its value is a printer property — declaring
#     `"Direct Drive Standard"` in a generic-PLA fragment risks
#     mis-routing on multi-extruder printers (H2D's Standard +
#     High Flow). The right shape is per-`when.extruder.variant`
#     rules; see `docs/profiles.md` → "Open: per-extruder cascade
#     resolution". Until that lands, just deny the key.
DENY_KEYS = frozenset({
    # Vestigial (libslic3r doesn't read these)
    "bed_type",
    "extruder_rotation_volume",
    "chamber_temperatures",
    "circle_compensation_speed",
    "counter_coef_1",
    "counter_coef_2",
    "counter_coef_3",
    "counter_limit_max",
    "counter_limit_min",
    # Pure UI / metadata
    "default_filament_colour",
    "filament_notes",
    "filament_printable",
    "compatible_printers_condition",
    "compatible_prints",
    "compatible_prints_condition",
    # Printer-binding leak — pending per-extruder cascade dim
    "filament_extruder_variant",
})

# Orca-side typo keys are folded to canonical at import — see
# `_profile_typos.fold_typo_keys`, shared across every profile importer.

# ---- Compatibility overrides for upstream authoring bugs ----
#
# libslic3r selects the bed-temperature key by the active plate's
# `curr_bed_type` (`Preset::get_bed_temp_key()` → `cool_plate_temp`,
# `eng_plate_temp`, `textured_plate_temp`, `hot_plate_temp`, …). Some
# upstream vendor profiles author the bed temp *only* under
# `hot_plate_temp`, even for printers whose physical/default plate maps
# to a different `curr_bed_type`. The active plate then reads the
# unauthored sibling key (`0`) and the bed never heats.
#
# These are genuine upstream bugs we can't fix in their tree, so we
# compensate at import time: for the affected printer, mirror the
# authored source plate-temp family into the target family that its
# `curr_bed_type` actually selects. Keyed by the `printer.model` string
# as it appears in `compatible_printers`; only fills a target that's
# absent or a zero sentinel, so a genuinely-authored value is never
# clobbered.
#
# Snapmaker U1: ships a swappable textured PEI plate (the only/default
# plate), so its bed identity resolves to `curr_bed_type = "Textured
# PEI Plate"` → `textured_plate_temp`. Upstream Snapmaker filament
# leaves only set `hot_plate_temp` and leave `textured_plate_temp = 0`
# (verified against the generated fragments). Mirror hot → textured.
COMPAT_PLATE_TEMP_MIRROR: dict[str, list[tuple[str, str]]] = {
    "Snapmaker U1": [
        ("hot_plate_temp", "textured_plate_temp"),
        ("hot_plate_temp_initial_layer", "textured_plate_temp_initial_layer"),
    ],
}


def _is_zero_temp(v: Any) -> bool:
    """A plate-temp value that means "unset" — empty, or every
    per-extruder element is `0`. Plate temps are vector-typed, so the
    value may be a scalar string or a list (`"0"`, `["0"]`, `["0","0"]`)."""
    elems = v if isinstance(v, list) else [v]
    return all(str(e).strip() in ("", "0") for e in elems)


def apply_compat_plate_temp_mirror(
    per_printer_values: dict[str, dict[str, Any]],
) -> None:
    """In-place: for each affected printer, copy the authored source
    plate-temp key into the target key its `curr_bed_type` selects,
    unless the target already carries a real (non-zero) value."""
    for printer_model, values in per_printer_values.items():
        mirrors = COMPAT_PLATE_TEMP_MIRROR.get(printer_model)
        if not mirrors:
            continue
        for src_key, dst_key in mirrors:
            src = values.get(src_key)
            if src is None or _is_zero_temp(src):
                continue
            dst = values.get(dst_key)
            if dst is None or _is_zero_temp(dst):
                values[dst_key] = list(src) if isinstance(src, list) else src


NOZZLE_SUFFIX_RE = re.compile(r" 0\.\d+ nozzle$")
SUFFIX_RE = re.compile(r" @(.+)$")
GENERIC_TOKEN_RE = re.compile(r"\bGeneric\b")

# Used to scrub the nozzle dimension from `compatible_printers`
# entries (and any other "printer.model" string we surface in
# `when.printer.model`). Matches a trailing decimal optionally
# followed by `mm` / `nozzle` / a parenthesized form. Decimal
# WITHOUT a `mm`/`nozzle`/paren indicator is NOT stripped — that
# would mis-strip printer names like `Prusa MK3.5`. Case-insensitive,
# handles 1.0/1.2/3.0 etc. (not just 0.X). Strips `HF0.4 nozzle` →
# `HF` (no leading space required before the decimal).
_COMPAT_NOZZLE_SUFFIX_RE = re.compile(
    r"\s*(?:\(\s*\d+\.\d+\s*(?:mm|nozzle)?\s*\)|\d+\.\d+\s*(?:mm|nozzle))\s*$",
    re.IGNORECASE,
)


def strip_compat_nozzle(s: str) -> str:
    """Strip a nozzle-suffix from a printer.model string surfaced
    in `compatible_printers`. Leaves names without a clear nozzle
    indicator alone."""
    return _COMPAT_NOZZLE_SUFFIX_RE.sub("", s).rstrip()


def normalize_value(v: Any) -> Any:
    """Collapse a list whose elements are all equal to a single-element
    list — semantically identical for libslic3r's per-extruder
    broadcasting, but unifies single-extruder leaves' `"0.98"` with
    multi-extruder leaves' `"0.98,0.98"` so they cluster instead of
    spawning extra rules. Leaves genuinely-divergent lists alone."""
    if isinstance(v, list) and len(v) > 1:
        first = v[0]
        if all(elem == first for elem in v):
            return [first]
    return v


# ---- Inheritance flattening ----

def build_filament_index(root: Path) -> dict[str, list[Path]]:
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


def resolve_parent(name: str, leaf_dir: Path, index: dict[str, list[Path]]) -> Path | None:
    candidates = index.get(name)
    if not candidates and "/" in name:
        for repl in ("-", " ", ""):
            candidates = index.get(name.replace("/", repl))
            if candidates:
                break
    if not candidates:
        return None
    leaf_root = leaf_dir
    while leaf_root.name != "filament" and leaf_root.parent != leaf_root:
        leaf_root = leaf_root.parent
    for c in candidates:
        c_root = c.parent
        while c_root.name != "filament" and c_root.parent != c_root:
            c_root = c_root.parent
        if c_root == leaf_root:
            return c
    return sorted(candidates)[0]


def flatten_leaf(leaf: Path, index: dict[str, list[Path]]) -> dict[str, Any] | None:
    chain: list[dict] = []
    visited: set[Path] = set()
    current = leaf
    while True:
        resolved = current.resolve()
        if resolved in visited:
            return None
        visited.add(resolved)
        try:
            doc = json.loads(current.read_text())
        except (json.JSONDecodeError, OSError):
            return None
        chain.append(doc)
        parent = doc.get("inherits")
        if not parent:
            break
        parent_path = resolve_parent(parent, current.parent, index)
        if parent_path is None:
            return None
        current = parent_path

    merged: dict[str, Any] = {}
    for doc in reversed(chain):
        for k, v in doc.items():
            if k in ENVELOPE_KEYS:
                continue
            merged[k] = v
    fold_typo_keys(merged)
    leaf_doc = chain[0]
    name = leaf_doc.get("name") or leaf.stem
    merged.setdefault("filament_settings_id", name)
    return merged


# ---- Logical-name extraction ----

_PRE_GENERIC_MODIFIERS = frozenset({"HF", "HS"})


def logical_name(leaf_stem: str, brand: str) -> str | None:
    """Extract the logical filament name from a leaf stem, scoped to
    `brand`.

    For `brand == "Generic"`: cross-vendor consolidation. Strips
    vendor prefix and pivots the `Creality HF Generic PLA` quirk so
    it buckets with the canonical `Generic <Material> HF` form.
    Returns None if the name doesn't contain a `Generic` token.
    Examples:
        `Dremel Generic PLA @3D20 all` → 'Generic PLA'
        `Generic PLA Silk @BBL H2D` → 'Generic PLA Silk'
        `Creality HF Generic PLA` → 'Generic PLA HF'

    For any other brand: branded consolidation. The full leaf name
    (minus @-suffix and trailing nozzle) IS the logical name — no
    vendor stripping, no Generic-token filtering. Each branded
    product is its own bucket.
    Examples (brand="Bambu Lab"):
        `Bambu PLA Basic @BBL A1` → 'Bambu PLA Basic'
        `Bambu PLA Matte @BBL X1C 0.4 nozzle` → 'Bambu PLA Matte'
    """
    m = SUFFIX_RE.search(leaf_stem)
    name = leaf_stem[:m.start()] if m else leaf_stem
    name = NOZZLE_SUFFIX_RE.sub("", name)
    if brand != "Generic":
        return name.strip() or None
    g = GENERIC_TOKEN_RE.search(name)
    if g is None:
        return None
    pre = name[:g.start()].strip()
    rest = name[g.start():].strip()
    if pre:
        words = pre.split()
        if words and words[-1] in _PRE_GENERIC_MODIFIERS:
            return f"{rest} {words[-1]}"
    return rest


def is_base_leaf(leaf_stem: str) -> bool:
    return leaf_stem.endswith(" @base")


def split_suffix(leaf_stem: str) -> tuple[str, str]:
    """`Generic PLA @BBL A1 0.2 nozzle` → ('Generic PLA', 'BBL A1 0.2 nozzle').
    For leaves without `@<suffix>`, returns (stem, '')."""
    m = SUFFIX_RE.search(leaf_stem)
    if not m:
        return leaf_stem, ""
    return leaf_stem[:m.start()], m.group(1)


_BARE_NOZZLE_RE = re.compile(r"(?<=[ @])0\.\d+(?: nozzle)?(?=$| )", re.IGNORECASE)


def strip_nozzle_anywhere(stem: str) -> str:
    """Remove `0.X nozzle` wherever it appears in the stem — at the
    end of the base (`Anker Generic ABS 0.25 nozzle`), inside an
    `@<suffix>` (`Generic PLA @BBL A1 0.2 nozzle`), or as a bare
    nozzle-only @-suffix (`Generic ABS @0.2 nozzle`). Also handles
    Prusa-style trailing `0.X` without the `nozzle` word."""
    stripped = _BARE_NOZZLE_RE.sub("", stem).rstrip()
    # Collapse a dangling `@` left when the @-suffix was nothing but
    # the nozzle (e.g. `Generic ABS @0.2 nozzle` → `Generic ABS @` → `Generic ABS`).
    if stripped.endswith(" @"):
        stripped = stripped[:-2]
    return stripped


def fold_nozzle_variants(leaves: list[Path]) -> list[Path]:
    """Collapse all leaves that differ only in nozzle suffix into one
    representative per (nozzle-stripped) stem. Prefer the no-nozzle
    leaf; fall back to `0.4 nozzle`; else alphabetical first.

    The nozzle suffix is a separate cascade dimension that
    `compatible_printers` doesn't carry, so retaining all nozzle
    variants pollutes per-printer deltas with nozzle-specific values
    (volumetric speed, pressure advance, retraction tuning). Handles
    both `<base> @<printer> 0.X nozzle` (the @-suffixed form) and
    `<base> 0.X nozzle.json` (bare, no `@`) shapes used by different
    vendors."""
    groups: dict[str, list[Path]] = defaultdict(list)
    for leaf in leaves:
        key = strip_nozzle_anywhere(leaf.stem)
        groups[key].append(leaf)

    chosen: list[Path] = []
    for key, members in groups.items():
        no_nozzle = [m for m in members if not _BARE_NOZZLE_RE.search(m.stem)]
        if no_nozzle:
            chosen.extend(sorted(no_nozzle))
            continue
        zero_four = [m for m in members if " 0.4 nozzle" in m.stem.lower()]
        if zero_four:
            chosen.append(sorted(zero_four)[0])
            continue
        chosen.append(sorted(members)[0])
    return chosen


def vendor_of_leaf(leaf: Path) -> str:
    """`<root>/<Vendor>/filament/.../foo.json` → 'Vendor'."""
    parts = leaf.parts
    for i, part in enumerate(parts):
        if part == "filament" and i > 0:
            return parts[i - 1]
    return ""


# ---- Printer-model expansion ----

def expand_printer_models(merged: dict[str, Any]) -> list[str]:
    """Walk `compatible_printers` and yield unique printer.model
    strings (nozzle dimension stripped). Empty list if missing/empty."""
    cp = merged.get("compatible_printers")
    if isinstance(cp, str):
        entries = [s.strip() for s in cp.split(",")]
    elif isinstance(cp, list):
        entries = [str(s).strip() for s in cp]
    else:
        return []
    seen: list[str] = []
    seen_set: set[str] = set()
    for e in entries:
        if not e:
            continue
        m = strip_compat_nozzle(e)
        if m and m not in seen_set:
            seen_set.add(m)
            seen.append(m)
    return seen


# ---- TOML emission ----

def _toml_string(s: str) -> str:
    if "\n" in s or "'''" in s:
        return '"""' + s.replace('\\', '\\\\').replace('"', '\\"') + '"""'
    if '"' in s or "\\" in s:
        return "'''" + s + "'''"
    return f'"{s}"'


def value_to_toml(v: Any) -> str:
    if isinstance(v, list):
        return _toml_string(",".join(str(x) for x in v))
    return _toml_string(str(v))


def slugify(name: str) -> str:
    """Normalize logical names to filesystem-safe slugs. `+` is
    expanded to `-plus-` first (so "Generic PLA+" → "generic-pla-plus"
    stays distinct from plain "Generic PLA")."""
    s = name.lower().replace("+", "-plus-")
    return re.sub(r"[^a-z0-9]+", "-", s).strip("-")


# ---- Baseline selection + delta computation ----

def _hashable(v: Any) -> Any:
    """Make a JSON-loaded value hashable so Counter can tally it."""
    if isinstance(v, list):
        return ("__list__", tuple(_hashable(x) for x in v))
    if isinstance(v, dict):
        return ("__dict__", tuple(sorted((k, _hashable(val)) for k, val in v.items())))
    return v


def _unhash(v: Any) -> Any:
    if isinstance(v, tuple) and v and v[0] == "__list__":
        return [_unhash(x) for x in v[1]]
    if isinstance(v, tuple) and v and v[0] == "__dict__":
        return {k: _unhash(val) for k, val in v[1]}
    return v


def find_majority_baseline(
    per_printer_values: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    """Pick the most-common value across all printers as baseline for
    each key. Ties broken by hash-stable ordering. Keys present in
    only a single printer end up as baseline = that one value (zero
    deltas)."""
    by_key: dict[str, Counter] = defaultdict(Counter)
    for values in per_printer_values.values():
        for key, value in values.items():
            by_key[key][_hashable(value)] += 1
    baseline: dict[str, Any] = {}
    for key, counter in by_key.items():
        most_common, _count = counter.most_common(1)[0]
        baseline[key] = _unhash(most_common)
    return baseline


def winner_is_new(
    existing_leaf: Path,
    existing_compat_count: int,
    new_leaf: Path,
    new_compat_count: int,
) -> bool:
    """Non-nozzle-suffixed beats nozzle-suffixed (the nozzle is a
    separate cascade dim; nozzle-keyed leaves are less authoritative
    as "the printer's recipe"). Then smaller compat list wins (more
    specific). Then BBL. Then alphabetical."""
    new_has_nozzle = bool(_BARE_NOZZLE_RE.search(new_leaf.stem))
    existing_has_nozzle = bool(_BARE_NOZZLE_RE.search(existing_leaf.stem))
    if new_has_nozzle != existing_has_nozzle:
        return existing_has_nozzle  # new wins iff existing has nozzle
    if new_compat_count != existing_compat_count:
        return new_compat_count < existing_compat_count
    new_is_bbl = vendor_of_leaf(new_leaf) == "BBL"
    existing_is_bbl = vendor_of_leaf(existing_leaf) == "BBL"
    if new_is_bbl != existing_is_bbl:
        return new_is_bbl
    return str(new_leaf) < str(existing_leaf)


# ---- Main consolidation pass ----

def consolidate_bucket(
    display_name: str,
    member_logicals: set[str],
    leaves: list[Path],
    root: Path,
    index: dict[str, list[Path]],
) -> tuple[dict[str, Any], dict[str, dict[str, Any]], list[str]] | None:
    """Returns (baseline_scalars, per_printer_deltas, conflict_log)
    for one bucket. Returns None if the bucket yields no per-printer
    values at all.

    Baseline is the most-common value per key across every
    (printer, leaf) entry — minimizes total rule volume vs picking
    any single leaf as canonical."""
    logical = display_name

    # Phase 1: collect per-(printer, key) values from all leaves,
    # resolving cross-leaf conflicts the same way as before (narrower
    # compat list wins; non-nozzle beats nozzle).
    chosen: dict[tuple[str, str], tuple[Any, Path, int]] = {}
    conflicts: list[str] = []

    for leaf in sorted(leaves):
        if is_base_leaf(leaf.stem):
            continue
        merged = flatten_leaf(leaf, index)
        if merged is None:
            continue
        printer_models = expand_printer_models(merged)
        if not printer_models:
            continue
        compat_count = len(printer_models)
        for printer_model in printer_models:
            for key, value in merged.items():
                if key in ENVELOPE_KEYS:
                    continue
                if key in DENY_KEYS:
                    continue
                if key == "compatible_printers":
                    continue
                if key == "filament_settings_id":
                    continue
                value = normalize_value(value)
                slot = (printer_model, key)
                existing = chosen.get(slot)
                if existing is None:
                    chosen[slot] = (value, leaf, compat_count)
                    continue
                existing_value, existing_leaf, existing_count = existing
                if existing_value == value:
                    if winner_is_new(existing_leaf, existing_count, leaf, compat_count):
                        chosen[slot] = (value, leaf, compat_count)
                    continue
                if winner_is_new(existing_leaf, existing_count, leaf, compat_count):
                    winner_value, winner_leaf = value, leaf
                    loser_value, loser_leaf = existing_value, existing_leaf
                    chosen[slot] = (value, leaf, compat_count)
                else:
                    winner_value, winner_leaf = existing_value, existing_leaf
                    loser_value, loser_leaf = value, leaf
                # Suppress nozzle-explosion noise.
                winner_has_nozzle = bool(_BARE_NOZZLE_RE.search(winner_leaf.stem))
                loser_has_nozzle = bool(_BARE_NOZZLE_RE.search(loser_leaf.stem))
                if winner_has_nozzle != loser_has_nozzle:
                    continue
                conflicts.append(
                    f"{logical} @ {printer_model} / {key}: "
                    f"{winner_value!r} ({winner_leaf.relative_to(root)}) wins, "
                    f"dropped {loser_value!r} ({loser_leaf.relative_to(root)})"
                )

    per_printer_full: dict[str, dict[str, Any]] = defaultdict(dict)
    for (printer_model, key), (value, _src, _cnt) in chosen.items():
        per_printer_full[printer_model][key] = value
    if not per_printer_full:
        return None

    # Compensate for upstream bed-temp authoring bugs before baseline
    # so the mirrored value rides the affected printer's own column
    # (e.g. U1's hot_plate_temp=55, not the bucket baseline) into the
    # right curr_bed_type key.
    apply_compat_plate_temp_mirror(per_printer_full)

    # Phase 2: most-common-value baseline per key.
    baseline_scalars = find_majority_baseline(per_printer_full)

    # Phase 3: per-printer rules carry only keys where the printer's
    # value differs from baseline.
    per_printer_deltas: dict[str, dict[str, Any]] = {}
    for printer_model, values in per_printer_full.items():
        deltas = {
            k: v for k, v in values.items()
            if baseline_scalars.get(k) != v
        }
        if deltas:
            per_printer_deltas[printer_model] = deltas

    return baseline_scalars, dict(per_printer_deltas), conflicts


def _toml_quoted(s: str) -> str:
    """Inline-array string element — TOML basic string with simple
    escapes. Used inside `[...]` array literals."""
    return '"' + s.replace('\\', '\\\\').replace('"', '\\"') + '"'


def _printer_array_literal(printer_models: list[str]) -> str:
    return "[" + ", ".join(_toml_quoted(p) for p in printer_models) + "]"


def emit_fragment(
    out_path: Path,
    logical: str,
    baseline_scalars: dict[str, Any],
    per_printer: dict[str, dict[str, Any]],
    leaf_count: int,
    printer_count: int,
) -> None:
    skip_keys = {"compatible_printers", "filament_settings_id"}
    baseline_keys = sorted(k for k in baseline_scalars if k not in skip_keys)

    # Group printers by identical full delta-set. Each group becomes
    # one multi-key rule with `when.printer.model = [...]` (OR-list)
    # so the cascade resolver fires once per matching printer.
    #
    # We tried richer biclustering and stripe-grouping approaches but
    # both were strictly worse on real consolidated data: printer-list
    # bytes dominate the cost, and per-printer-group is the unique
    # representation that never duplicates a printer name across
    # rules. Sticking with the simpler form.
    full_groups: dict[tuple, list[str]] = defaultdict(list)
    for printer_model, deltas in per_printer.items():
        if not deltas:
            continue
        signature = tuple(sorted(
            (k, _hashable(v)) for k, v in deltas.items()
        ))
        full_groups[signature].append(printer_model)
    groups: list[tuple[frozenset[str], list[tuple[str, Any]]]] = [
        (frozenset(printers), list(signature))
        for signature, printers in full_groups.items()
    ]
    # Sort by printer-count descending so the broadest rule appears
    # first when eyeballing the fragment.
    groups.sort(key=lambda g: (-len(g[0]), sorted(g[0])[0] if g[0] else ""))

    lines: list[str] = []
    lines.append(f"# {logical} — consolidated filament fragment.")
    lines.append("# Generated by scripts/import_filaments.py.")
    lines.append(f"# {leaf_count} upstream leaf/leaves consolidated; "
                 f"{printer_count} printer variant(s) covered; "
                 f"{len(groups)} delta group(s).")
    lines.append("")
    lines.append("# ---- Baseline (most-common value per key) ----")
    lines.append(f'filament_settings_id = "{logical}"')
    for k in baseline_keys:
        lines.append(f"{k} = {value_to_toml(baseline_scalars[k])}")
    lines.append("")

    if groups:
        lines.append("# ---- Per-printer overrides (deltas vs baseline) ----")
        # Emit in the greedy-pick order — the first rule is the
        # largest rectangle and corresponds to "most printers agree
        # on the most keys", which usually reads as the most
        # interpretable rule first.
        for printer_set, key_values in groups:
            printer_models = sorted(printer_set)
            lines.append("")
            lines.append("[[rule]]")
            if len(printer_models) == 1:
                lines.append(f'when.printer.model = "{printer_models[0]}"')
            else:
                lines.append(
                    f"when.printer.model = {_printer_array_literal(printer_models)}"
                )
            for key, hashable_value in sorted(key_values):
                lines.append(f"set.{key} = {value_to_toml(_unhash(hashable_value))}")
        lines.append("")

    atomic_write_text(out_path, "\n".join(lines))


# ---- Main ----

def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--root", type=Path, required=True,
                    help="profiles tree root (e.g. external/OrcaSlicer/resources/profiles)")
    ap.add_argument("--out", type=Path, required=True,
                    help="output directory (e.g. resources/profiles/generic/filament)")
    ap.add_argument("--brand", default="Generic",
                    help="filament_vendor value to filter leaves by, also "
                         "selects the bucket-extraction strategy. 'Generic' "
                         "(default) does cross-vendor consolidation with "
                         "vendor-prefix stripping; any other value does "
                         "branded consolidation, using the full leaf name "
                         "as the logical bucket.")
    args = ap.parse_args()

    root = args.root.resolve()
    if not root.is_dir():
        sys.exit(f"error: --root {root} is not a directory")

    index = build_filament_index(root)

    # Walk every `filament/` subtree, bucket leaves by SLUG (so name
    # punctuation variants like "Generic TPU 95A" vs "Generic TPU-95A"
    # collapse into one bucket).
    buckets: dict[str, list[Path]] = defaultdict(list)
    bucket_names: dict[str, set[str]] = defaultdict(set)
    scanned = 0
    skipped_non_generic = 0
    skipped_unflattenable = 0
    for vendor_dir in sorted(root.iterdir()):
        if not vendor_dir.is_dir():
            continue
        filament_root = vendor_dir / "filament"
        if not filament_root.is_dir():
            continue
        for leaf in filament_root.rglob("*.json"):
            scanned += 1
            logical = logical_name(leaf.stem, args.brand)
            if logical is None:
                continue  # bucket extraction rejected it
            merged = flatten_leaf(leaf, index)
            if merged is None:
                skipped_unflattenable += 1
                continue
            vendors = merged.get("filament_vendor")
            if not (isinstance(vendors, list) and vendors == [args.brand]):
                skipped_non_generic += 1
                continue
            slug = slugify(logical)
            buckets[slug].append(leaf)
            bucket_names[slug].add(logical)

    # Per-bucket: fold nozzle-suffixed variants into their no-nozzle
    # parent (or 0.4 fallback) before delta computation.
    for slug in list(buckets):
        buckets[slug] = fold_nozzle_variants(buckets[slug])

    print(f"scanned {scanned} filament leaf JSONs")
    print(f"  skipped non-generic: {skipped_non_generic}")
    print(f"  skipped unflattenable: {skipped_unflattenable}")
    print(f"  generic buckets: {len(buckets)}")

    args.out.mkdir(parents=True, exist_ok=True)
    all_conflicts: list[str] = []
    written = 0
    for slug in sorted(buckets):
        leaves = buckets[slug]
        member_logicals = bucket_names[slug]
        # Display name: BBL's logical wins (likely the cleanest form),
        # otherwise pick the alphabetically-first member.
        bbl_logicals = sorted({
            logical_name(p.stem, args.brand)
            for p in leaves
            if vendor_of_leaf(p) == "BBL" and logical_name(p.stem, args.brand) is not None
        })
        display_name = bbl_logicals[0] if bbl_logicals else sorted(member_logicals)[0]
        result = consolidate_bucket(display_name, member_logicals, leaves, root, index)
        if result is None:
            print(f"  SKIP {slug} ({display_name!r}): no per-printer values",
                  file=sys.stderr)
            continue
        baseline_scalars, per_printer, conflicts = result
        # Total printer count covered: union of all per-leaf
        # compatible_printers expansions, regardless of whether the
        # printer's values matched baseline (zero-delta printers don't
        # appear in per_printer).
        all_printers: set[str] = set()
        for leaf in leaves:
            if is_base_leaf(leaf.stem):
                continue
            merged = flatten_leaf(leaf, index)
            if merged is None:
                continue
            all_printers.update(expand_printer_models(merged))
        out_path = args.out / f"{slug}.toml"
        emit_fragment(
            out_path, display_name,
            baseline_scalars,
            per_printer, len(leaves), len(all_printers),
        )
        merge_note = ""
        if len(member_logicals) > 1:
            other_names = sorted(member_logicals - {display_name})
            merge_note = f" (merged from {len(member_logicals)} names: +{', '.join(other_names)})"
        print(f"  {slug}: {len(leaves)} leaves, "
              f"{len(all_printers)} printers ({len(per_printer)} with deltas), "
              f"{len(conflicts)} conflict(s){merge_note}")
        all_conflicts.extend(conflicts)
        written += 1

    print(f"\nwrote {written} fragments to {args.out}")

    if all_conflicts:
        print(f"\n{len(all_conflicts)} conflict(s) logged:", file=sys.stderr)
        for c in all_conflicts:
            print(f"  {c}", file=sys.stderr)


if __name__ == "__main__":
    main()
