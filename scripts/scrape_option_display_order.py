#!/usr/bin/env python3
"""Scrape OrcaSlicer's UI display order for ConfigOptionDef keys.

OrcaSlicer's settings panels (TabPrint / TabFilament / TabPrinter) are
hand-curated in `src/slic3r/GUI/Tab.cpp`: each Tab calls
`add_options_page(L("Page"), ...)`, then `page->new_optgroup(...)`,
then `optgroup->append_single_option_line("KEY", ...)` to lay rows
out in their intended display order.

PrintConfig.cpp's OPT_PTR registration order — which `option_defs()`
returns — is *not* this display order; upstream registers options
roughly alphabetically / by code-organization. Reading the
registration order as the UI order puts `raft_layers` before
`enable_support` in the Support category, which is wrong.

This scraper produces a Rust lookup table mapping each option key to
its first-encountered position in Tab.cpp. The settings panel sorts
options by this position so the rendered order matches Orca's panel.

Keys absent from Tab.cpp (internal scratch options, deprecated keys,
etc.) get `None` from the lookup; callers sort them to the end via a
stable sort fallback.

Run after pulling upstream OrcaSlicer:

    python3 scripts/scrape_option_display_order.py

The output is committed to the repo, not regenerated at cargo time.
"""
from __future__ import annotations

import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
TAB_CPP = REPO_ROOT / "external/OrcaSlicer/src/slic3r/GUI/Tab.cpp"
OUTPUT_PATH = REPO_ROOT / "crates/slic3r-ffi/src/option_display_order.rs"

# `append_single_option_line("KEY", ...)` — the canonical form.
# Allow whitespace around the open paren and either `->` or `.`
# before the call so we catch `optgroup->append_single_option_line(...)`
# as well as the rare bare form.
APPEND_LITERAL = re.compile(
    r'append_single_option_line\s*\(\s*"([A-Za-z_][A-Za-z0-9_]*)"'
)


def scrape(text: str) -> list[str]:
    """Return option keys in first-encounter order across Tab.cpp."""
    seen: dict[str, None] = {}
    for match in APPEND_LITERAL.finditer(text):
        seen.setdefault(match.group(1), None)
    return list(seen.keys())


def emit_rust(keys: list[str]) -> str:
    # Sort by key for binary_search; carry the (key, position) pair so
    # the position itself is what we use to sort options at runtime.
    indexed = sorted(((k, i) for i, k in enumerate(keys)), key=lambda kv: kv[0])
    body = "\n".join(f'    ("{k}", {i}),' for k, i in indexed)
    return f"""//! Per-key UI display-order position, scraped from OrcaSlicer's
//! `src/slic3r/GUI/Tab.cpp`.
//!
//! AUTO-GENERATED — do not edit by hand. Regenerate with
//! `scripts/scrape_option_display_order.py` after pulling new
//! upstream OrcaSlicer source. Positions are the first-encounter
//! index of each option key across TabPrint/TabFilament/TabPrinter,
//! preserved exactly as Orca's hand-curated `append_single_option_line`
//! calls lay them out.
//!
//! Keys absent from this table (internal-only options, deprecated
//! fields, etc.) return `None`; callers should sort them last via a
//! stable-sort fallback on registration order.

const DISPLAY_ORDER: &[(&str, u32)] = &[
{body}
];

/// First-encounter position of the option key in Orca's settings UI,
/// or `None` for keys that don't appear in any Tab page.
pub fn display_order_of(key: &str) -> Option<u32> {{
    DISPLAY_ORDER
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| DISPLAY_ORDER[i].1)
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn binary_search_relies_on_sorted_keys() {{
        let mut last = "";
        for (key, _) in DISPLAY_ORDER {{
            assert!(*key > last, "DISPLAY_ORDER must be sorted; {{key}} <= {{last}}");
            last = key;
        }}
    }}

    #[test]
    fn support_section_order_matches_orca() {{
        // Orca's Tab.cpp Support page opens with `enable_support`,
        // then `support_type`, `support_style`, … and only much later
        // gets to `raft_layers` (separate Raft optgroup). The relative
        // ordering of these keys is the canary that catches a
        // mis-scrape.
        let pos = |k: &str| display_order_of(k).expect(k);
        assert!(pos("enable_support") < pos("support_type"));
        assert!(pos("support_type") < pos("support_style"));
        assert!(pos("support_style") < pos("support_threshold_angle"));
        assert!(pos("support_threshold_angle") < pos("raft_layers"));
    }}

    #[test]
    fn unknown_key_returns_none() {{
        assert!(display_order_of("totally_made_up_option").is_none());
    }}
}}
"""


def main() -> None:
    text = TAB_CPP.read_text()
    keys = scrape(text)
    if not keys:
        raise SystemExit(
            f"error: no append_single_option_line matches in {TAB_CPP.relative_to(REPO_ROOT)}"
        )
    OUTPUT_PATH.write_text(emit_rust(keys))
    print(
        f"wrote {OUTPUT_PATH.relative_to(REPO_ROOT)}: {len(keys)} keys "
        f"from {TAB_CPP.relative_to(REPO_ROOT)}"
    )


if __name__ == "__main__":
    main()
