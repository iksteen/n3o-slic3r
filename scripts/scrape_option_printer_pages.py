#!/usr/bin/env python3
"""Scrape OrcaSlicer's printer-settings page layout for option keys.

The Process/Filament settings carry a `category` on their ConfigOptionDef,
so the settings panel can group them (Quality, Strength, Speed, …). The
*printer* (machine) options almost all carry an empty category — their
grouping lives only in `src/slic3r/GUI/Tab.cpp`'s `TabPrinter`, which lays
them out across pages via:

    add_options_page(L("Page title"), icon);
    optgroup = page->new_optgroup(L("Group"));
    optgroup->append_single_option_line("KEY");

This scraper walks the `TabPrinter::` member functions (the FFF build,
the kinematics page, and the dynamically-built unregular pages — but not
the SLA build) and maps each machine-option key to the page it first
appears under. The machine-settings panel uses that page as the option's
category, reproducing Orca's printer-settings tabs instead of the flat
"Machine limits + everything else" the bare `category` field would give.

Keys reached only through array/loop construction (the `machine_max_*`
families on the "Motion ability" page) are caught when they appear as
string literals; the few that don't fall back to the libslic3r `category`
on the consuming side.

Run after pulling upstream OrcaSlicer:

    python3 scripts/scrape_option_printer_pages.py

The output is committed to the repo, not regenerated at cargo time.
"""
from __future__ import annotations

import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
TAB_CPP = REPO_ROOT / "external/OrcaSlicer/src/slic3r/GUI/Tab.cpp"
PRINT_CONFIG_CPP = REPO_ROOT / "external/OrcaSlicer/src/libslic3r/PrintConfig.cpp"
OUTPUT_PATH = REPO_ROOT / "crates/slic3r-ffi/src/option_printer_pages.rs"

# The authoritative per-extruder option set is libslic3r's own
# `m_extruder_option_keys` (PrintConfig.cpp::init_extruder_option_keys) —
# the keys whose vectors are sized to the extruder count. The GUI's
# extruder-page loop is only a *view* of these (and omits some, e.g.
# `extruder_colour`, whose widget is commented out), so we read the data
# model, not Tab.cpp, for the membership.
EXTRUDER_KEYS_BLOCK = re.compile(
    r"m_extruder_option_keys\s*=\s*\{(.*?)\}", re.DOTALL
)
QUOTED_KEY = re.compile(r'"([a-z][a-z0-9_]*)"')

# A C++ member-function definition at column 0, e.g.
# `void TabPrinter::build_fff()` or `PageShp TabPrinter::build_kinematics_page()`.
FUNC_DEF = re.compile(r"^[A-Za-z][\w:<>*&\s]*?\bTab(\w+)::(\w+)\s*\(")

# The current page / optgroup title.
PAGE = re.compile(r'add_options_page\(\s*L\("([^"]+)"\)')
OPTGROUP = re.compile(r'new_optgroup\(\s*(?:L\()?\s*"([^"]+)"')

# Option-key references we trust to be real keys (not labels/icons).
KEY_FORMS = [
    re.compile(r'append_single_option_line\(\s*"([a-z][a-z0-9_]*)"'),
    re.compile(r'get_option\(\s*"([a-z][a-z0-9_]*)"'),
    re.compile(r'create_line_with_widget\([^,]+,\s*"([a-z][a-z0-9_]*)"'),
]


def scrape(text: str) -> tuple[dict[str, str], dict[str, str]]:
    """Map each printer-option key to its nav category and sub-group.

    Orca's layout is page → optgroup → rows. For machine-wide keys the
    nav category is the page (Basic information, Machine G-code, …) and the
    sub-group is the optgroup (Printable space, Advanced, …). Per-extruder
    keys (those Orca lays out in the `extruder_idx` loop) use the optgroup
    as the nav category and have no sub-group — they're grouped per toolhead
    on their own tab. Per-extruder *membership* comes from PrintConfig.cpp
    (`parse_extruder_keys`), not this loop."""
    category_of: dict[str, str] = {}
    subgroup_of: dict[str, str] = {}
    in_printer = False  # inside a TabPrinter:: function we care about
    current_page = ""
    current_group = ""

    for line in text.splitlines():
        m = FUNC_DEF.match(line)
        if m:
            cls, method = m.group(1), m.group(2)
            # Capture across all TabPrinter FFF/kinematics/unregular pages;
            # the SLA build groups SLA-only keys we never surface.
            in_printer = cls == "Printer" and method != "build_sla"
            current_page = current_group = ""
            continue
        if not in_printer:
            continue
        p = PAGE.search(line)
        if p:
            current_page = p.group(1)
            current_group = ""
        g = OPTGROUP.search(line)
        if g:
            current_group = g.group(1)
        is_extruder = "extruder_idx" in line
        # Per-extruder keys group by optgroup; the rest by page (with the
        # optgroup as a sub-group within that page).
        category = current_group if is_extruder else current_page
        if not category:
            continue
        for form in KEY_FORMS:
            for key in form.findall(line):
                if key in category_of:
                    continue
                category_of[key] = category
                if not is_extruder and current_group:
                    subgroup_of[key] = current_group
    return category_of, subgroup_of


def parse_extruder_keys(print_config_text: str) -> set[str]:
    """The authoritative per-extruder key set from libslic3r's
    `m_extruder_option_keys` initializer."""
    m = EXTRUDER_KEYS_BLOCK.search(print_config_text)
    if not m:
        raise SystemExit(
            "error: m_extruder_option_keys block not found in "
            f"{PRINT_CONFIG_CPP.relative_to(REPO_ROOT)}"
        )
    return set(QUOTED_KEY.findall(m.group(1)))


def emit_rust(
    category_of: dict[str, str],
    subgroup_of: dict[str, str],
    per_extruder: set[str],
) -> str:
    rows = sorted(category_of.items(), key=lambda kv: kv[0])
    body = "\n".join(f'    ("{k}", "{c}"),' for k, c in rows)
    sub_rows = sorted(subgroup_of.items(), key=lambda kv: kv[0])
    sub_body = "\n".join(f'    ("{k}", "{g}"),' for k, g in sub_rows)
    ex_body = "\n".join(f'    "{k}",' for k in sorted(per_extruder))
    return f"""//! Per-key printer-settings category + per-extruder flag, scraped from
//! OrcaSlicer.
//!
//! AUTO-GENERATED — do not edit by hand. Regenerate with
//! `scripts/scrape_option_printer_pages.py` after pulling new upstream
//! OrcaSlicer source.
//!
//! Category (from `src/slic3r/GUI/Tab.cpp` `TabPrinter`): the
//! `add_options_page` title the key appears under (Basic information,
//! Machine G-code, …) for machine-wide options, or the `new_optgroup`
//! title (Retraction, Z-Hop, …) for keys in the extruder-page loop —
//! printer options carry no libslic3r `category` of their own. Keys absent
//! from the table return `None`; callers fall back to an "Other" bucket.
//!
//! Per-extruder set (from `src/libslic3r/PrintConfig.cpp`
//! `m_extruder_option_keys`): the authoritative list of options sized to
//! the extruder count. Sourced from the data model, not the GUI, because
//! Orca's extruder-page widgets omit some members (e.g. `extruder_colour`,
//! whose widget is commented out). These render one tab per toolhead.

const PRINTER_PAGES: &[(&str, &str)] = &[
{body}
];

const PRINTER_SUBGROUPS: &[(&str, &str)] = &[
{sub_body}
];

const PER_EXTRUDER: &[&str] = &[
{ex_body}
];

/// The printer-settings category the option key appears under in Orca's
/// `TabPrinter` (page for machine-wide keys, optgroup for per-extruder
/// keys), or `None` for keys not laid out there.
pub fn printer_page_of(key: &str) -> Option<&'static str> {{
    PRINTER_PAGES
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| PRINTER_PAGES[i].1)
}}

/// The optgroup (sub-section within a page) a machine-wide option appears
/// under — e.g. "Printable space" under the "Basic information" page. The
/// panel renders these as sub-headers within the page. `None` for keys with
/// no sub-group (per-extruder keys, or keys not laid out in Tab.cpp).
pub fn printer_subgroup_of(key: &str) -> Option<&'static str> {{
    PRINTER_SUBGROUPS
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| PRINTER_SUBGROUPS[i].1)
}}

/// True if the option is laid out per-extruder (one value per toolhead)
/// in Orca's `TabPrinter` — the set the per-extruder UI tabs surface.
pub fn is_per_extruder(key: &str) -> bool {{
    PER_EXTRUDER.binary_search(&key).is_ok()
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn tables_are_sorted_for_binary_search() {{
        let mut last = "";
        for (key, _) in PRINTER_PAGES {{
            assert!(*key > last, "PRINTER_PAGES must be sorted; {{key}} <= {{last}}");
            last = key;
        }}
        let mut last = "";
        for (key, _) in PRINTER_SUBGROUPS {{
            assert!(*key > last, "PRINTER_SUBGROUPS must be sorted; {{key}} <= {{last}}");
            last = key;
        }}
        let mut last = "";
        for key in PER_EXTRUDER {{
            assert!(*key > last, "PER_EXTRUDER must be sorted; {{key}} <= {{last}}");
            last = key;
        }}
    }}

    #[test]
    fn known_keys_map_to_their_orca_category() {{
        assert_eq!(printer_page_of("gcode_flavor"), Some("Basic information"));
        assert_eq!(printer_page_of("machine_start_gcode"), Some("Machine G-code"));
        assert_eq!(printer_page_of("retraction_length"), Some("Retraction"));
        // Sub-group within a page: z_offset sits under "Printable space".
        assert_eq!(printer_subgroup_of("z_offset"), Some("Printable space"));
        assert_eq!(printer_subgroup_of("gcode_flavor"), Some("Advanced"));
    }}

    #[test]
    fn per_extruder_flag_matches_libslic3r_set() {{
        assert!(is_per_extruder("retraction_length"));
        assert!(is_per_extruder("z_hop"));
        assert!(is_per_extruder("nozzle_diameter"));
        // In libslic3r's set even though Orca's extruder-page widget for
        // it is commented out.
        assert!(is_per_extruder("extruder_colour"));
        // Machine-wide keys are not per-extruder.
        assert!(!is_per_extruder("gcode_flavor"));
        assert!(!is_per_extruder("machine_start_gcode"));
    }}

    #[test]
    fn unknown_key_returns_none() {{
        assert!(printer_page_of("totally_made_up_option").is_none());
        assert!(!is_per_extruder("totally_made_up_option"));
    }}
}}
"""


def main() -> None:
    category_of, subgroup_of = scrape(TAB_CPP.read_text())
    if not category_of:
        raise SystemExit(
            f"error: no TabPrinter option keys found in {TAB_CPP.relative_to(REPO_ROOT)}"
        )
    per_extruder = parse_extruder_keys(PRINT_CONFIG_CPP.read_text())
    OUTPUT_PATH.write_text(emit_rust(category_of, subgroup_of, per_extruder))
    cats = sorted(set(category_of.values()))
    print(
        f"wrote {OUTPUT_PATH.relative_to(REPO_ROOT)}: {len(category_of)} keys "
        f"across {len(cats)} categories; {len(subgroup_of)} with sub-groups; "
        f"{len(per_extruder)} per-extruder keys (from libslic3r m_extruder_option_keys)"
    )


if __name__ == "__main__":
    main()
