#!/usr/bin/env python3
"""Scrape OrcaSlicer's per-bucket option-key vectors into a Rust source file.

OrcaSlicer's `Preset.cpp` partitions every ConfigOptionDef into one of three
buckets — Printer / Filament / Process — via hand-curated C++ string vectors:

    Preset::print_options()    = s_Preset_print_options
    Preset::filament_options() = s_Preset_filament_options
    Preset::printer_options()  = s_Preset_printer_options
                               + s_Preset_machine_limits_options
                               + Preset::nozzle_options() (= m_extruder_option_keys)

This script parses those static vectors out of the C++ source and emits a Rust
data table that the FFI can consume to tag each OptionDef with its bucket.

Run this when the upstream OrcaSlicer source is updated:

    python3 scripts/scrape_option_buckets.py

The output is committed to the repo; we don't regenerate at build time.
"""
from __future__ import annotations

import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
PRESET_CPP = REPO_ROOT / "external/OrcaSlicer/src/libslic3r/Preset.cpp"
PRINT_CONFIG_CPP = REPO_ROOT / "external/OrcaSlicer/src/libslic3r/PrintConfig.cpp"
OUTPUT_PATH = REPO_ROOT / "crates/slic3r-ffi/src/option_buckets.rs"

STRING_LITERAL = re.compile(r'"([A-Za-z_][A-Za-z0-9_]*)"')


def extract_block(text: str, start_marker: str) -> str:
    """Return the C++ source between `start_marker` and the matching `};`.

    The vector initializers are well-formed `{ ... };` blocks with no nested
    braces that would confuse a brace-counter (they only contain string
    literals + comments).
    """
    idx = text.find(start_marker)
    if idx < 0:
        raise RuntimeError(f"marker not found: {start_marker!r}")
    end = text.find("};", idx)
    if end < 0:
        raise RuntimeError(f"unterminated block starting at marker: {start_marker!r}")
    return text[idx:end]


def extract_keys(block: str) -> list[str]:
    """Pull every C++ string literal out of `block`, in order, deduped."""
    seen: dict[str, None] = {}
    for match in STRING_LITERAL.finditer(block):
        seen[match.group(1)] = None
    return list(seen.keys())


def main() -> None:
    preset_src = PRESET_CPP.read_text()
    print_config_src = PRINT_CONFIG_CPP.read_text()

    print_block = extract_block(preset_src, "s_Preset_print_options{")
    filament_block = extract_block(preset_src, "s_Preset_filament_options {")
    printer_block = extract_block(preset_src, "s_Preset_printer_options {")
    machine_limits_block = extract_block(preset_src, "s_Preset_machine_limits_options {")
    extruder_block = extract_block(print_config_src, "m_extruder_option_keys = {")

    print_keys = extract_keys(print_block)
    filament_keys = extract_keys(filament_block)
    printer_keys = (
        extract_keys(printer_block)
        + extract_keys(machine_limits_block)
        + extract_keys(extruder_block)
    )
    # Dedupe printer keys (extruder set overlaps with printer set).
    printer_keys = list(dict.fromkeys(printer_keys))

    # Keys that appear in multiple buckets are per-preset metadata
    # (compatible_printers, inherits, …) — not real config options.
    # Drop them from the bucket table; bucket_of() will return None and the
    # settings panel will skip them (correct UX — they're not user-editable).
    sets = {
        "Process": set(print_keys),
        "Filament": set(filament_keys),
        "Printer": set(printer_keys),
    }
    meta_keys: set[str] = set()
    for a, b in [("Process", "Filament"), ("Process", "Printer"), ("Filament", "Printer")]:
        meta_keys |= sets[a] & sets[b]
    if meta_keys:
        print(f"meta keys (omitted, appear in multiple buckets): {sorted(meta_keys)}")

    print_keys = [k for k in print_keys if k not in meta_keys]
    filament_keys = [k for k in filament_keys if k not in meta_keys]
    printer_keys = [k for k in printer_keys if k not in meta_keys]

    entries: list[tuple[str, str]] = []
    for key in print_keys:
        entries.append((key, "Process"))
    for key in filament_keys:
        entries.append((key, "Filament"))
    for key in printer_keys:
        entries.append((key, "Printer"))
    # Global sort (binary_search_by_key needs this).
    entries.sort(key=lambda kv: kv[0])

    # Emit Rust source.
    body = "\n".join(
        f'    ("{key}", OptBucket::{bucket}),' for key, bucket in entries
    )
    out = f"""//! Per-key bucket classification, scraped from OrcaSlicer's Preset.cpp.
//!
//! AUTO-GENERATED — do not edit by hand. Regenerate with
//! `scripts/scrape_option_buckets.py` after pulling new upstream OrcaSlicer
//! source. Buckets mirror `Preset::printer_options()` / `filament_options()`
//! / `print_options()` exactly; the partitioning is hand-curated upstream.
//!
//! Keys not present in this table fall outside the FFF preset universe
//! (typically SLA-only or internal scratch fields).

use crate::OptBucket;

const OPTION_BUCKETS: &[(&str, OptBucket)] = &[
{body}
];

/// Bucket for the given OptionDef key, or `None` for keys not partitioned by
/// `Preset::*_options()` (SLA-only, internal, etc.).
pub fn bucket_of(key: &str) -> Option<OptBucket> {{
    OPTION_BUCKETS
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| OPTION_BUCKETS[i].1)
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn binary_search_relies_on_sorted_keys() {{
        let mut last = "";
        for (key, _) in OPTION_BUCKETS {{
            assert!(*key > last, "OPTION_BUCKETS must be sorted; {{key}} <= {{last}}");
            last = key;
        }}
    }}

    #[test]
    fn known_keys_resolve() {{
        assert_eq!(bucket_of("layer_height"),     Some(OptBucket::Process));
        assert_eq!(bucket_of("nozzle_diameter"),  Some(OptBucket::Printer));
        assert_eq!(bucket_of("nozzle_temperature"), Some(OptBucket::Filament));
        assert_eq!(bucket_of("filament_type"),    Some(OptBucket::Filament));
        assert_eq!(bucket_of("machine_max_speed_x"), Some(OptBucket::Printer));
        assert_eq!(bucket_of("nonexistent_key"),  None);
    }}
}}
"""
    OUTPUT_PATH.write_text(out)

    total = len(print_keys) + len(filament_keys) + len(printer_keys)
    print(
        f"wrote {OUTPUT_PATH.relative_to(REPO_ROOT)}: "
        f"{len(print_keys)} process + {len(filament_keys)} filament + "
        f"{len(printer_keys)} printer = {total} keys"
    )


if __name__ == "__main__":
    main()
