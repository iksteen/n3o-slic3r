#!/usr/bin/env python3
"""Convert one (machine, filament, process) triple from an OrcaSlicer
vendor tree into three per-bucket cascade fragments.

This is the PR-S-4 successor to `scripts/spikes/convert_bbs_profile.py`.
Where the older script emits one monolithic cascade per printer
(merging all keys regardless of bucket), this script emits three
files per triple:

  <out_dir>/printer/<machine-slug>.toml      ← printer-bucket keys
  <out_dir>/filament/<filament-slug>.toml    ← filament-bucket keys
  <out_dir>/process/<process-slug>.toml      ← process-bucket keys

Each fragment is a plain cascade TOML with top-level key/value pairs
(the default rule the loader implicitly synthesizes). No `[[rule]]`
blocks — plate-dimensional adjustments are already encoded as
libslic3r-style per-plate arrays and the runtime adapter handles them.

The bucket partition comes from `crates/slic3r-ffi/src/option_buckets.rs`
which is itself scraped from OrcaSlicer's `Preset.cpp` by
`scripts/scrape_option_buckets.py`. Keys without a bucket assignment
(SLA-only, internal scratch, etc.) are dropped with a warning so the
output stays clean.

Run example:

    scripts/convert_per_bucket.py \\
        --vendor BBL \\
        --machine "Bambu Lab A1 mini 0.4 nozzle" \\
        --filament "Bambu PLA Basic @BBL A1M" \\
        --process "0.20mm Standard @BBL A1M" \\
        --out-dir profiles/vendor/bbl

The PR-S-5 runtime composer will compose these fragments at slice time
in the order (printer → filament[slot] → process → plate_overrides →
object_overrides).
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# Reuse the inheritance walker + BBS filter from the legacy converter.
# Both scripts live under scripts/ and import each other directly;
# keeping the helpers in one place avoids drift between the per-bucket
# fragments and the monolithic cascade during the PR-S-4→PR-S-5
# transition.
sys.path.insert(0, str(Path(__file__).resolve().parent / "spikes"))
from convert_bbs_profile import (  # type: ignore[import-not-found]
    apply_bbs_filter,
    find_by_name,
    flatten_inheritance,
    value_to_toml,
)

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_PROFILES_ROOT = REPO_ROOT / "external/OrcaSlicer/resources/profiles"
OPTION_BUCKETS_RS = REPO_ROOT / "crates/slic3r-ffi/src/option_buckets.rs"

BUCKET_RX = re.compile(r'\("([^"]+)",\s*OptBucket::(\w+)\)')


def _safe_rel(p: Path) -> Path:
    """Return `p` relative to the repo root when possible, otherwise
    `p` unchanged. Lets the converter write to /tmp during testing
    without crashing the print line."""
    try:
        return p.relative_to(REPO_ROOT)
    except ValueError:
        return p


def load_buckets() -> dict[str, str]:
    """Parse `option_buckets.rs` into `{key: 'printer'|'filament'|'process'}`.

    The Rust file is a generated static table; format is stable:

        const OPTION_BUCKETS: &[(&str, OptBucket)] = &[
            ("alternate_extra_wall", OptBucket::Process),
            ...
        ];
    """
    text = OPTION_BUCKETS_RS.read_text()
    out: dict[str, str] = {}
    for match in BUCKET_RX.finditer(text):
        key, bucket = match.group(1), match.group(2).lower()
        out[key] = bucket
    if not out:
        sys.exit(
            f"no entries parsed from {OPTION_BUCKETS_RS} — "
            "regenerate it with scripts/scrape_option_buckets.py"
        )
    return out


def slugify(name: str) -> str:
    """File-safe slug from a vendor profile display name.

    `"Bambu Lab A1 mini 0.4 nozzle"` → `"bambu-lab-a1-mini-0.4-nozzle"`.
    `@`, `(`, `)`, etc. collapse to a single hyphen; leading/trailing
    hyphens are stripped. Dots are kept (nozzle diameter notation).
    """
    s = name.lower()
    s = re.sub(r"[@()_/\s]+", "-", s)
    s = re.sub(r"-+", "-", s).strip("-")
    return s


def partition_by_bucket(
    merged_dicts: dict[str, dict],
    buckets: dict[str, str],
) -> tuple[dict[str, dict], list[tuple[str, str]]]:
    """Split keys across the per-bucket output dicts.

    `merged_dicts` is `{"machine": …, "filament": …, "process": …}` —
    each value the post-BBS-filter merged-inheritance dict for that
    source. The output is `({"printer": {…}, "filament": {…},
    "process": {…}}, unknown_keys)`.

    A key's bucket is determined by [`load_buckets`] — NOT by which
    source dict it appeared in. BBS sometimes sets a printer-bucket key
    from inside a filament profile (or vice versa); the bucket
    classification from `Preset.cpp` is the authority.

    Duplicate keys (same key in multiple source dicts) take the value
    from the highest-priority source: machine wins over filament wins
    over process — mirroring upstream's per-bucket preset precedence.
    """
    out: dict[str, dict] = {"printer": {}, "filament": {}, "process": {}}
    unknown: list[tuple[str, str]] = []
    # Iteration order = priority: machine → filament → process. The
    # `setdefault` below preserves the first writer's value.
    for source in ("machine", "filament", "process"):
        for key, value in merged_dicts[source].items():
            bucket = buckets.get(key)
            if bucket is None:
                unknown.append((source, key))
                continue
            out[bucket].setdefault(key, value)
    return out, unknown


def write_fragment(
    out_path: Path,
    bucket: str,
    payload: dict,
    provenance: dict,
) -> None:
    """Write one per-bucket fragment to disk.

    The fragment is a flat TOML — every key becomes a top-level
    assignment, which the cascade loader implicitly treats as the
    default rule (no `when` predicate).
    """
    lines: list[str] = []
    lines.append(f"# {provenance['display_name']} — {bucket} bucket")
    lines.append("#")
    lines.append("# AUTO-GENERATED by scripts/convert_per_bucket.py from:")
    lines.append(f"#   {provenance['source_path']}")
    lines.append("# Do not edit by hand. Regenerate when upstream changes.")
    lines.append("")
    for key in sorted(payload):
        lines.append(f"{key} = {value_to_toml(payload[key])}")
    lines.append("")

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text("\n".join(lines))


def main() -> None:
    p = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    p.add_argument("--vendor", required=True,
                   help="Vendor directory under --profiles-root (e.g. BBL, Snapmaker)")
    p.add_argument("--machine", required=True,
                   help="Machine profile display name (e.g. 'Bambu Lab A1 mini 0.4 nozzle')")
    p.add_argument("--filament", required=True,
                   help="Filament profile display name (e.g. 'Bambu PLA Basic @BBL A1M')")
    p.add_argument("--process", required=True,
                   help="Process profile display name (e.g. '0.20mm Standard @BBL A1M')")
    p.add_argument("--out-dir", required=True, type=Path,
                   help="Output directory; printer/, filament/, process/ subdirs are created")
    p.add_argument("--profiles-root", type=Path, default=DEFAULT_PROFILES_ROOT,
                   help=f"Vendor tree root (default: {DEFAULT_PROFILES_ROOT.relative_to(REPO_ROOT)})")
    args = p.parse_args()

    vendor_dir = args.profiles_root / args.vendor
    if not vendor_dir.is_dir():
        sys.exit(f"vendor directory missing: {vendor_dir}")

    buckets = load_buckets()

    # Walk inheritance + apply BBS rename/drop filter for each input.
    sources = {
        "machine": (find_by_name(vendor_dir, "machine", args.machine), args.machine),
        "filament": (find_by_name(vendor_dir, "filament", args.filament), args.filament),
        "process": (find_by_name(vendor_dir, "process", args.process), args.process),
    }

    merged: dict[str, dict] = {}
    for kind, (path, _name) in sources.items():
        raw = flatten_inheritance(path, kind, vendor_dir)
        filtered, _dropped, _renamed = apply_bbs_filter(raw)
        merged[kind] = filtered

    partitioned, unknown = partition_by_bucket(merged, buckets)

    # Emit fragments. Slug + filename comes from the source's display name.
    slugs = {
        "printer": slugify(args.machine),
        "filament": slugify(args.filament),
        "process": slugify(args.process),
    }
    source_paths = {
        "printer": _safe_rel(sources["machine"][0]),
        "filament": _safe_rel(sources["filament"][0]),
        "process": _safe_rel(sources["process"][0]),
    }
    display_names = {
        "printer": args.machine,
        "filament": args.filament,
        "process": args.process,
    }

    # `*_settings_id` keys are runtime-required by Bambu firmware
    # (they're read from the gcode CONFIG_BLOCK to validate the slice
    # against what's in the preset library — an empty filament_settings_id
    # triggers an immediate "Print cancelled"). They aren't in
    # `Preset::*_options()` so they have no OptBucket assignment, but each
    # belongs naturally to one of our fragments: the value is the preset's
    # display name. Inject them at fragment-write time so the per-bucket
    # output is self-contained.
    partitioned["printer"]["printer_settings_id"] = args.machine
    partitioned["filament"]["filament_settings_id"] = args.filament
    partitioned["process"]["print_settings_id"] = args.process

    for bucket in ("printer", "filament", "process"):
        out_path = args.out_dir / bucket / f"{slugs[bucket]}.toml"
        provenance = {
            "display_name": display_names[bucket],
            "source_path": source_paths[bucket],
        }
        write_fragment(out_path, bucket, partitioned[bucket], provenance)
        print(
            f"wrote {_safe_rel(out_path)} "
            f"({len(partitioned[bucket])} keys)"
        )

    # Known-handled keys that the BBS converter routes specially:
    # - *_settings_id are runtime-injected from preset names above.
    # - filament_ids is the runtime-assembled per-slot vector (the
    #   PR-S-5 composer derives it from each bound slot's filament_id;
    #   we don't store it in the fragment).
    KNOWN_HANDLED = {
        "printer_settings_id", "filament_settings_id", "print_settings_id",
        "filament_ids",
    }
    truly_unknown = [(s, k) for (s, k) in unknown if k not in KNOWN_HANDLED]
    if truly_unknown:
        print(f"\n{len(truly_unknown)} unrecognized keys (no OptBucket assignment, dropped):")
        for source, key in sorted(truly_unknown):
            print(f"  [{source}] {key}")


if __name__ == "__main__":
    main()
