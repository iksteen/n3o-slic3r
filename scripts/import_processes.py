#!/usr/bin/env python3
"""Consolidate upstream Orca/BBS process leaves into per-process-name
cascade fragments, with per-(printer, nozzle) rules.

Input shape — per-(layer-height × tier × printer × nozzle) leaves:

    BBL/process/0.20mm Standard @BBL A1M.json
    BBL/process/0.20mm Standard @BBL A1M 0.2 nozzle.json
    BBL/process/0.30mm Standard @BBL A1M 0.6 nozzle.json
    BBL/process/0.20mm Standard @BBL X1C.json
    Snapmaker/process/0.20 Standard @Snapmaker U1 (0.4 nozzle).json
    ...

Output shape — one cascade fragment per logical process name:

    profiles/process/0.20mm-standard.toml
    profiles/process/0.30mm-standard.toml
    profiles/process/0.20-standard.toml          (Snapmaker naming)
    ...

Each fragment groups every upstream leaf sharing the leaf-stem before
the `@<printer>` suffix. So "0.20mm Standard @BBL A1M",
"0.20mm Standard @BBL A1M 0.2 nozzle", and "0.20mm Standard @BBL X1C"
all fold into `0.20mm-standard.toml`, with per-(printer, nozzle)
deltas as rules.

Algorithm:

  1. Walk upstream `<vendor>/process/*.json` leaves (with `@<suffix>`
     — base/parent files are skipped). Filter to leaves whose
     `compatible_printers` contains at least one entry whose
     nozzle-stripped form matches `--printer-models`.

  2. Group by slug of the leaf stem with `@<suffix>` removed
     (e.g. "0.20mm Standard"). Same name across printers folds.

  3. For each leaf:
     - Flatten its inheritance chain (cross-vendor parent lookup,
       same as `import_filaments.py`).
     - For each `compatible_printers` entry that matches a supported
       printer, parse out (printer.model, nozzle.diameter) and
       record the leaf's flattened keys against that key.

  4. Within the bucket:
     - Baseline = most-common value per key across all (printer,
       nozzle) entries.
     - Per-(printer, nozzle) deltas = keys differing from baseline.
     - Per-nozzle OR-grouping: printers within the same nozzle that
       share an identical delta set merge into one rule with
       `when.printer.model = [...]`.

  5. Emit each rule with both `when.nozzle.diameter` and
     `when.printer.model` predicates so the cascade resolver fires
     only on the matching (printer, nozzle) pair at slice time.

  6. Apply the same per-extruder array collapse (`"X,X" → "X"`)
     and structural-noise deny logic the filament consolidator uses.

Usage:

    import_processes.py \\
        --root external/OrcaSlicer/resources/profiles \\
        --profiles-root profiles

The scope of supported printers and their on-disk locations comes
from walking `<profiles-root>/vendor/*/printer/*/machine.toml` and
reading the `printer_model` field. Each consolidated fragment is
written to every compatible printer's `processes/` dir.

Existing files in those `processes/` dirs are NOT removed
automatically — wipe them yourself before re-running if the bucket
shape has changed.
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

# Keys that don't belong in generated process fragments. Trimmed
# compared to filaments since processes already speak slice-side
# vocabulary; the main category errors are UI hints and metadata
# fields. Extend as more leak.
DENY_KEYS = frozenset({
    "print_settings_id",  # backfilled at import time, identity field
    "compatible_printers_condition",
    "compatible_prints_condition",
})

SUFFIX_RE = re.compile(r" @(.+)$")
# Match the nozzle suffix in compatible_printers entries. Same broad
# rules as the filament consolidator: accept `0.X nozzle`,
# `0.X Nozzle`, `1.0 nozzle`, `(0.X nozzle)`, `(0.X)`, etc. Requires
# either `mm`/`nozzle` word or parens, to avoid mis-stripping printer
# model names that legitimately end in a decimal (Prusa MK3.5).
_COMPAT_NOZZLE_RE = re.compile(
    r"\s*(?:\(\s*(\d+\.\d+(?:\+\d+\.\d+)?)\s*(?:mm|nozzle)?\s*\)"
    r"|(\d+\.\d+(?:\+\d+\.\d+)?)\s*(?:mm|nozzle))\s*$",
    re.IGNORECASE,
)


# ---- Inheritance flattening (lifted shape from import_filaments.py) ----

def build_process_index(root: Path) -> dict[str, list[Path]]:
    index: dict[str, list[Path]] = {}
    for vendor_dir in root.iterdir():
        if not vendor_dir.is_dir():
            continue
        process_root = vendor_dir / "process"
        if not process_root.is_dir():
            continue
        for p in process_root.rglob("*.json"):
            index.setdefault(p.stem, []).append(p)
    return index


def resolve_parent(name: str, leaf_dir: Path, index: dict[str, list[Path]]) -> Path | None:
    candidates = index.get(name)
    if not candidates:
        return None
    leaf_root = leaf_dir
    while leaf_root.name != "process" and leaf_root.parent != leaf_root:
        leaf_root = leaf_root.parent
    for c in candidates:
        c_root = c.parent
        while c_root.name != "process" and c_root.parent != c_root:
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
    leaf_doc = chain[0]
    name = leaf_doc.get("name") or leaf.stem
    merged.setdefault("print_settings_id", name)
    return merged


# ---- Logical name + (printer, nozzle) extraction ----

def logical_process_name(stem: str) -> str:
    """`0.20mm Standard @BBL A1M` → '0.20mm Standard'."""
    m = SUFFIX_RE.search(stem)
    return stem[:m.start()] if m else stem


# Strip the layer-height prefix from a process display name to get
# its tier ("0.20mm Standard" → "Standard", "0.20 Standard" → "Standard").
# BBL uses "0.20mm", Snapmaker uses "0.20" — accept both.
_TIER_PREFIX_RE = re.compile(r"^\d+(?:\.\d+)?\s*(?:mm)?\s+", re.IGNORECASE)


def tier_of(display_name: str) -> str:
    return _TIER_PREFIX_RE.sub("", display_name).strip()


def parse_printer_and_nozzle(s: str) -> tuple[str, str] | None:
    """`Bambu Lab A1 mini 0.4 nozzle` → ('Bambu Lab A1 mini', '0.4')
    `Snapmaker U1 (0.4 nozzle)` → ('Snapmaker U1', '0.4')
    `Snapmaker U1 (0.4+0.6 nozzle)` → ('Snapmaker U1', '0.4+0.6')

    The nozzle string is preserved as-found (so multi-nozzle profiles
    like '0.4+0.6' come through as a literal cascade dim value;
    picker matches against composite-nozzle contexts)."""
    m = _COMPAT_NOZZLE_RE.search(s)
    if not m:
        return None
    nozzle = m.group(1) or m.group(2)
    printer = s[:m.start()].rstrip()
    return printer, nozzle


def normalize_value(v: Any) -> Any:
    """Collapse all-equal-element lists to single-element."""
    if isinstance(v, list) and len(v) > 1:
        first = v[0]
        if all(elem == first for elem in v):
            return [first]
    return v


def _hashable(v: Any) -> Any:
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


def _toml_quoted(s: str) -> str:
    return '"' + s.replace('\\', '\\\\').replace('"', '\\"') + '"'


def _printer_array_literal(printer_models: list[str]) -> str:
    return "[" + ", ".join(_toml_quoted(p) for p in printer_models) + "]"


def slugify(name: str) -> str:
    return re.sub(r"[^a-z0-9.]+", "-", name.lower()).strip("-")


# ---- Consolidation ----

def consolidate_bucket(
    display_name: str,
    leaves: list[Path],
    supported_printers: set[str],
    index: dict[str, list[Path]],
) -> tuple[dict[str, Any],
           dict[tuple[str, str], dict[str, Any]],
           list[tuple[str, str]],
           list[str]] | None:
    """Returns (baseline_scalars, per_pn_deltas, available_for,
    conflict_log) where per_pn_deltas[(printer.model, nozzle.diameter)]
    is a dict of key→value deltas vs baseline, and available_for is
    the full set of (printer, nozzle) tuples this process applies to
    — including those whose values match baseline entirely (no
    rule). The picker uses available_for; the resolver uses the
    rules."""
    # Per-(printer, nozzle) → {key: value} matrix.
    per_pn: dict[tuple[str, str], dict[str, Any]] = {}
    conflicts: list[str] = []

    for leaf in sorted(leaves):
        merged = flatten_leaf(leaf, index)
        if merged is None:
            continue
        cp = merged.get("compatible_printers")
        if isinstance(cp, str):
            cp_list = [s.strip() for s in cp.split(",")]
        elif isinstance(cp, list):
            cp_list = [str(s).strip() for s in cp]
        else:
            cp_list = []

        for entry in cp_list:
            parsed = parse_printer_and_nozzle(entry)
            if parsed is None:
                # Entry without nozzle suffix — printer.model only.
                # Default nozzle convention: BBL = 0.4 (the "default"
                # in upstream naming when no nozzle suffix on the leaf).
                if entry in supported_printers:
                    parsed = (entry, "0.4")
                else:
                    continue
            printer, nozzle = parsed
            if printer not in supported_printers:
                continue
            slot = per_pn.setdefault((printer, nozzle), {})
            for key, value in merged.items():
                if key in ENVELOPE_KEYS or key in DENY_KEYS:
                    continue
                if key == "compatible_printers":
                    continue
                value = normalize_value(value)
                if key in slot and slot[key] != value:
                    conflicts.append(
                        f"{display_name} @ ({printer}, {nozzle}) / {key}: "
                        f"{slot[key]!r} (from earlier leaf) vs "
                        f"{value!r} (from {leaf.name}) — kept first"
                    )
                    continue
                slot[key] = value

    if not per_pn:
        return None

    # Most-common-value baseline across every (printer, nozzle) entry.
    by_key: dict[str, Counter] = defaultdict(Counter)
    for values in per_pn.values():
        for key, value in values.items():
            by_key[key][_hashable(value)] += 1
    baseline: dict[str, Any] = {}
    for key, counter in by_key.items():
        most_common, _ = counter.most_common(1)[0]
        baseline[key] = _unhash(most_common)

    # Per-(printer, nozzle) deltas.
    per_pn_deltas: dict[tuple[str, str], dict[str, Any]] = {}
    for pn, values in per_pn.items():
        deltas = {k: v for k, v in values.items() if baseline.get(k) != v}
        if deltas:
            per_pn_deltas[pn] = deltas
    available_for = sorted(per_pn.keys())
    return baseline, per_pn_deltas, available_for, conflicts


def emit_fragment(
    out_path: Path,
    logical_name: str,
    baseline: dict[str, Any],
    per_pn_deltas: dict[tuple[str, str], dict[str, Any]],
    available_for: list[tuple[str, str]],
    leaf_count: int,
) -> None:
    skip_keys = {"compatible_printers", "print_settings_id"}
    baseline_keys = sorted(k for k in baseline if k not in skip_keys)

    # Group every available (printer, nozzle) pair — including
    # baseline-only ones (no deltas) — into a (nozzle,
    # delta-signature) → [printer, …] map. Baseline-only pairs land
    # with an empty signature; their rule block carries the `when`
    # predicates with no `set.*` lines, which is how the cascade
    # records "this process is compatible with (P, N)" without
    # repeating the baseline. The picker derives compatibility by
    # walking these rule predicates — there's no separate `[meta]`
    # block anymore.
    by_nozzle: dict[str, dict[tuple, list[str]]] = defaultdict(
        lambda: defaultdict(list)
    )
    for printer, nozzle in available_for:
        deltas = per_pn_deltas.get((printer, nozzle), {})
        sig = tuple(sorted((k, _hashable(v)) for k, v in deltas.items()))
        by_nozzle[nozzle][sig].append(printer)

    nozzles_covered = sorted({n for (_, n) in available_for})
    printers_covered = sorted({p for (p, _) in available_for})

    lines: list[str] = []
    lines.append(f"# {logical_name} — consolidated process fragment.")
    lines.append("# Generated by scripts/import_processes.py.")
    lines.append(f"# {leaf_count} upstream leaf/leaves consolidated; "
                 f"{len(printers_covered)} printer(s) × {len(nozzles_covered)} nozzle(s).")
    lines.append("")
    lines.append("# ---- Baseline (most-common value per key) ----")
    lines.append(f'print_settings_id = "{logical_name}"')
    for k in baseline_keys:
        lines.append(f"{k} = {value_to_toml(baseline[k])}")
    lines.append("")

    lines.append("# ---- Per-(printer, nozzle) compatibility + overrides ----")
    lines.append("# Every (printer, nozzle) pair this process supports has at")
    lines.append("# least one [[rule]] below; pairs that match the baseline")
    lines.append("# carry a `when`-only rule (no `set.*` lines). The picker")
    lines.append("# derives availability from these predicates.")
    for nozzle in sorted(by_nozzle.keys()):
        # Sort groups within nozzle: larger printer-sets first,
        # then alphabetical by first printer.
        groups = sorted(
            by_nozzle[nozzle].items(),
            key=lambda kv: (-len(kv[1]), sorted(kv[1])[0]),
        )
        for sig, printers in groups:
            printers = sorted(printers)
            lines.append("")
            lines.append("[[rule]]")
            lines.append(f'when.nozzle.diameter = "{nozzle}"')
            if len(printers) == 1:
                lines.append(f'when.printer.model = "{printers[0]}"')
            else:
                lines.append(
                    f"when.printer.model = {_printer_array_literal(printers)}"
                )
            for k, hv in sig:
                lines.append(f"set.{k} = {value_to_toml(_unhash(hv))}")

    atomic_write_text(out_path, "\n".join(lines) + "\n")


def backfill_nozzle_default_process(
    nozzle_path: Path, default_slug: str | None,
) -> bool:
    """Edit `default_process_profile = "..."` on a hand-curated
    `nozzles/<sku>.toml`.

    Mirrors `import_machine_profile.py`'s
    `update_model_toml_default_bed`: preserves all other content,
    comments, and layout. Inserts after the existing
    `default_filament_profile = ...` line when the key is missing;
    replaces the line in place if present; strips it when
    `default_slug is None`. Returns True iff the file changed."""
    text = nozzle_path.read_text(encoding="utf-8")
    pattern = re.compile(
        r"^default_process_profile\s*=.*(?:\r?\n|$)",
        re.MULTILINE,
    )
    if default_slug is None:
        new_text, n = pattern.subn("", text)
        if n == 0:
            return False
    else:
        replacement = f'default_process_profile = "{default_slug}"\n'
        if pattern.search(text):
            new_text = pattern.sub(replacement, text, count=1)
        else:
            # Insert after default_filament_profile if present,
            # otherwise at the top of the body. Matches the natural
            # adjacency: both are "defaults the cascade composer
            # picks for a fresh instance."
            anchor = re.compile(
                r"^default_filament_profile\s*=.*(?:\r?\n|$)",
                re.MULTILINE,
            )
            m = anchor.search(text)
            if m:
                new_text = text[: m.end()] + replacement + text[m.end():]
            else:
                new_text = replacement + text
    if new_text == text:
        return False
    atomic_write_text(nozzle_path, new_text)
    return True


# ---- Main ----

def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--root", type=Path, required=True,
                    help="profiles tree root (e.g. external/OrcaSlicer/resources/profiles)")
    ap.add_argument("--profiles-root", type=Path, required=True,
                    help="our profiles tree root (e.g. profiles/). The "
                         "consolidator walks `<profiles-root>/vendor/*/printer/*/"
                         "machine.toml` to discover printer.model → printer-dir "
                         "mappings; each consolidated fragment is written into "
                         "every compatible printer's `processes/` dir.")
    args = ap.parse_args()

    root = args.root.resolve()
    if not root.is_dir():
        sys.exit(f"error: --root {root} is not a directory")
    profiles_root = args.profiles_root.resolve()
    if not profiles_root.is_dir():
        sys.exit(f"error: --profiles-root {profiles_root} is not a directory")

    # Discover printer.model → printer-dir map by walking machine.toml
    # files. Authoritative — no convention parsing.
    printer_dir_by_model: dict[str, Path] = {}
    for machine_path in (profiles_root / "vendor").glob("*/printer/*/machine.toml"):
        for line in machine_path.read_text().splitlines():
            if line.startswith("printer_model"):
                # `printer_model = "Bambu Lab A1 mini"` — split on `=`
                # and strip quotes/whitespace.
                _, _, rhs = line.partition("=")
                model = rhs.strip().strip('"')
                if model:
                    printer_dir_by_model[model] = machine_path.parent
                break
    if not printer_dir_by_model:
        sys.exit(f"error: no machine.toml files found under "
                 f"{profiles_root}/vendor/*/printer/*/")
    supported_printers = set(printer_dir_by_model)
    print(f"discovered {len(supported_printers)} printer profile(s): "
          f"{sorted(supported_printers)}")

    index = build_process_index(root)

    # Discover all process leaves with an @-suffix (skipping
    # inheritance parents like fdm_process_common.json), bucket by
    # logical name.
    buckets: dict[str, list[Path]] = defaultdict(list)
    scanned = 0
    skipped_no_match = 0
    for vendor_dir in sorted(root.iterdir()):
        if not vendor_dir.is_dir():
            continue
        proc_root = vendor_dir / "process"
        if not proc_root.is_dir():
            continue
        for leaf in proc_root.rglob("*.json"):
            scanned += 1
            stem = leaf.stem
            if " @" not in stem:
                continue  # base / inheritance parent
            # Quick filter: any compatible_printers entry maps to a
            # supported printer?
            try:
                doc = json.loads(leaf.read_text())
            except (json.JSONDecodeError, OSError):
                continue
            cp = doc.get("compatible_printers")
            entries = cp if isinstance(cp, list) else (
                [cp] if isinstance(cp, str) else []
            )
            matches = False
            for entry in entries:
                parsed = parse_printer_and_nozzle(str(entry).strip())
                if parsed is None:
                    continue
                if parsed[0] in supported_printers:
                    matches = True
                    break
            if not matches:
                skipped_no_match += 1
                continue
            buckets[slugify(logical_process_name(stem))].append(leaf)

    print(f"scanned {scanned} process leaf JSONs; "
          f"{skipped_no_match} skipped (no supported-printer match); "
          f"{len(buckets)} buckets")

    written_files = 0
    written_buckets = 0
    all_conflicts: list[str] = []
    # (printer_model, nozzle_spec) → slug of the "Standard" tier
    # fragment for that combo. Populated as we consolidate; used
    # after the loop to backfill `default_process_profile` into the
    # matching nozzle.toml files. Composite specs (containing `+`)
    # are excluded since they have no single nozzle.toml to write
    # into — they'd go in machine.toml's [meta] in a future round.
    standard_default_for: dict[tuple[str, str], str] = {}
    multi_standard_conflicts: list[str] = []
    for slug in sorted(buckets):
        leaves = buckets[slug]
        # Pick a display name: the lexicographically first leaf's
        # logical name (sortable, stable).
        display_name = sorted({
            logical_process_name(p.stem) for p in leaves
        })[0]
        result = consolidate_bucket(
            display_name, leaves, supported_printers, index,
        )
        if result is None:
            print(f"  SKIP {slug}: no matching (printer, nozzle) entries",
                  file=sys.stderr)
            continue
        baseline, per_pn_deltas, available_for, conflicts = result
        # Track this fragment as the Standard for each (printer,
        # single-nozzle) combo it covers. If two Standards target
        # the same combo, that's a curation-needs-attention case
        # (upstream variance — see report); first wins, conflict
        # logged at the end.
        if tier_of(display_name).lower() == "standard":
            for printer, nozzle in available_for:
                if "+" in nozzle:
                    continue  # composite — out of scope for single nozzle.toml
                key = (printer, nozzle)
                if key in standard_default_for:
                    multi_standard_conflicts.append(
                        f"{printer} / {nozzle} nozzle: both "
                        f"`{standard_default_for[key]}` and `{slug}` "
                        f"are Standard-tier; keeping `{standard_default_for[key]}`"
                    )
                else:
                    standard_default_for[key] = slug
        # Write a copy into every applicable printer's processes/ dir.
        # Same content per copy — the cascade resolver narrows to the
        # active (printer, nozzle) via the `when.printer.model` /
        # `when.nozzle.diameter` predicates inside the file.
        target_printers = sorted({p for (p, _) in available_for})
        for printer in target_printers:
            printer_dir = printer_dir_by_model.get(printer)
            if printer_dir is None:
                continue
            out_path = printer_dir / "processes" / f"{slug}.toml"
            emit_fragment(
                out_path, display_name, baseline, per_pn_deltas,
                available_for, len(leaves),
            )
            written_files += 1
        nozzles_count = len({n for (_, n) in available_for})
        delta_rules = sum(len(d) > 0 for d in per_pn_deltas.values())
        print(f"  {slug}: {len(leaves)} leaves, "
              f"{len(target_printers)} printer(s) × {nozzles_count} nozzle(s), "
              f"{delta_rules} delta cell(s), "
              f"{len(conflicts)} conflict(s)")
        all_conflicts.extend(conflicts)
        written_buckets += 1

    print(f"\nwrote {written_buckets} fragments → {written_files} file(s) "
          f"across {len(printer_dir_by_model)} printer dir(s)")

    # Backfill `default_process_profile` into each nozzle.toml. The
    # picker's default-selection rule (rule 1: each nozzle profile
    # registers its default process) reads from there at runtime.
    # We only do single-nozzle files here — composite-nozzle defaults
    # need a separate spot (machine.toml [meta]) and stay future work.
    backfill_count = 0
    backfill_unchanged = 0
    backfill_missing = 0
    for (printer, nozzle), slug in sorted(standard_default_for.items()):
        printer_dir = printer_dir_by_model.get(printer)
        if printer_dir is None:
            continue
        nozzle_path = printer_dir / "nozzles" / f"{nozzle}.toml"
        if not nozzle_path.is_file():
            backfill_missing += 1
            print(
                f"  backfill: {printer} / {nozzle} → `{slug}` "
                f"(no nozzle.toml found at {nozzle_path.relative_to(profiles_root)})",
                file=sys.stderr,
            )
            continue
        if backfill_nozzle_default_process(nozzle_path, slug):
            backfill_count += 1
            print(f"  backfill: {nozzle_path.relative_to(profiles_root)} ← `{slug}`")
        else:
            backfill_unchanged += 1
    print(
        f"\nbackfilled default_process_profile into {backfill_count} "
        f"nozzle.toml file(s) ({backfill_unchanged} unchanged, "
        f"{backfill_missing} missing target)"
    )
    if multi_standard_conflicts:
        print(
            f"\n{len(multi_standard_conflicts)} (printer, nozzle) combos have "
            f"multiple Standard-tier fragments (first wins for backfill):",
            file=sys.stderr,
        )
        for c in multi_standard_conflicts:
            print(f"  {c}", file=sys.stderr)

    if all_conflicts:
        print(f"\n{len(all_conflicts)} conflict(s):", file=sys.stderr)
        for c in all_conflicts[:30]:
            print(f"  {c}", file=sys.stderr)
        if len(all_conflicts) > 30:
            print(f"  ... + {len(all_conflicts) - 30} more", file=sys.stderr)


if __name__ == "__main__":
    main()
