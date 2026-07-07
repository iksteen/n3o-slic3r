#!/usr/bin/env python3
"""Scrape OrcaSlicer's printer- and filament-settings page layout.

The Process options carry a `category` on their ConfigOptionDef, so the
settings panel groups them (Quality, Strength, Speed, …). The *printer*
(machine) and *filament* options almost all carry an empty category —
their grouping lives only in `src/slic3r/GUI/Tab.cpp`'s `TabPrinter` /
`TabFilament`, which lay them out across pages via:

    add_options_page(L("Page title"), icon);
    optgroup = page->new_optgroup(L("Group"));
    optgroup->append_single_option_line("KEY");
    // …or, for multi-option lines:
    line.append_option(optgroup->get_option("KEY"));

This scraper walks each tab's member functions and maps every option key
to the page (and optgroup) it first appears under, so the machine- and
filament-settings panels reproduce Orca's tabs instead of the flat
"everything in Other" the bare `category` field would give.

Run after pulling upstream OrcaSlicer:

    python3 scripts/scrape_option_printer_pages.py

The output is committed to the repo, not regenerated at cargo time.
"""
from __future__ import annotations

import re
from pathlib import Path

from _orca_tab_keys import make_append_tracker

REPO_ROOT = Path(__file__).resolve().parent.parent
TAB_CPP = REPO_ROOT / "external/OrcaSlicer/src/slic3r/GUI/Tab.cpp"
OUTPUT_PATH = REPO_ROOT / "crates/slic3r-ffi/src/option_printer_pages.rs"

# A C++ member-function definition at column 0, e.g.
# `void TabPrinter::build_fff()` or `PageShp TabPrinter::build_kinematics_page()`.
FUNC_DEF = re.compile(r"^[A-Za-z][\w:<>*&\s]*?\bTab(\w+)::(\w+)\s*\(")

# The current page / optgroup title.
PAGE = re.compile(r'add_options_page\(\s*L\("([^"]+)"\)')
OPTGROUP = re.compile(r'new_optgroup\(\s*(?:L\()?\s*"([^"]+)"')

# A multi-option line with its own label, e.g.
# `line = { L("Cool Plate"), L("tooltip") };` — the label is the row
# identifier (plate type, "Nozzle", …) for the options appended to it, which
# otherwise share generic labels ("Other layers" / "First layer"). Reset at
# `optgroup->append_line(line)`. TabPrinter also declares freshly-typed line
# vars (`Line resonance_line = { L("Resonance Avoidance Speed"), … };`) whose
# members carry only generic labels ("Min"/"Max", "X"/"Y") — the line label is
# the only thing that gives them meaning.
LINE_START = re.compile(r'\bline\s*=\s*\{\s*L\("([^"]+)"')
LINE_DECL = re.compile(r'\bLine\s+\w+\s*=\s*\{\s*L\("([^"]+)"')
APPEND_LINE = re.compile(r"append_line\(")

# Option-key references we trust to be real keys (not labels/icons).
KEY_FORMS = [
    re.compile(r'append_single_option_line\(\s*"([a-z][a-z0-9_]*)"'),
    re.compile(r'get_option\(\s*"([a-z][a-z0-9_]*)"'),
    re.compile(r'create_line_with_widget\([^,]+,\s*"([a-z][a-z0-9_]*)"'),
]


def scrape(
    text: str, tab_class: str, extruder_aware: bool
) -> tuple[dict[str, str], dict[str, str], dict[str, str]]:
    """Map each option key in `Tab<tab_class>` to its nav category, sub-group
    and (when it sits on a labeled multi-option line) that line's label.

    Orca's layout is page → optgroup → rows. The nav category is the page
    (Basic information, Print temperature, …) and the sub-group is the
    optgroup. When `extruder_aware` (the printer tab), keys laid out in the
    `extruder_idx` loop instead use the optgroup as their nav category and
    carry no sub-group — they're grouped per toolhead on their own tab.
    Per-extruder *membership* comes from PrintConfig.cpp, not this loop. The
    line label disambiguates options whose own label is generic (the bed-temp
    plate types all share "Other layers" / "First layer")."""
    category_of: dict[str, str] = {}
    subgroup_of: dict[str, str] = {}
    line_of: dict[str, str] = {}
    in_target = False  # inside a Tab<tab_class>:: function we care about
    current_page = ""
    current_group = ""
    current_line = ""
    # Resolves the machine_max_* keys Orca builds via append_option_line +
    # loop-variable / string concat (invisible to the literal KEY_FORMS).
    append_keys = make_append_tracker()

    def record(key: str, is_extruder: bool) -> None:
        if key in category_of:
            return
        category_of[key] = current_group if is_extruder else current_page
        if not is_extruder and current_group:
            subgroup_of[key] = current_group
        if not is_extruder and current_line:
            line_of[key] = current_line

    for line in text.splitlines():
        m = FUNC_DEF.match(line)
        if m:
            cls, method = m.group(1), m.group(2)
            # Capture across the tab's FFF/kinematics/unregular page
            # builders; the SLA build groups SLA-only keys we never surface.
            in_target = cls == tab_class and method != "build_sla"
            current_page = current_group = current_line = ""
            continue
        if not in_target:
            continue
        p = PAGE.search(line)
        if p:
            current_page = p.group(1)
            current_group = current_line = ""
        g = OPTGROUP.search(line)
        if g:
            current_group = g.group(1)
            current_line = ""
        ls = LINE_START.search(line) or LINE_DECL.search(line)
        if ls:
            current_line = ls.group(1)
        is_extruder = extruder_aware and "extruder_idx" in line
        # Per-extruder keys group by optgroup; the rest by page (with the
        # optgroup as a sub-group within that page).
        category = current_group if is_extruder else current_page
        # Fed every line so its vector-literal / loop-binding state stays
        # continuous; keys only surface on the append_option_line calls.
        append_line_keys = append_keys(line)
        if category:
            for form in KEY_FORMS:
                for key in form.findall(line):
                    record(key, is_extruder)
            # append_option_line keys land in the current optgroup too — the
            # Motion ability page's speed/acceleration/jerk limitation groups.
            for key in append_line_keys:
                record(key, is_extruder)
        # A labeled line closes here; following single-option lines carry
        # their own labels.
        if APPEND_LINE.search(line):
            current_line = ""
    return category_of, subgroup_of, line_of


def _table(name: str, rows: dict[str, str]) -> str:
    body = "\n".join(f'    ("{k}", "{c}"),' for k, c in sorted(rows.items()))
    return f"const {name}: &[(&str, &str)] = &[\n{body}\n];"


def _lookup_fn(fn: str, table: str, doc: str) -> str:
    return f"""/// {doc}
pub fn {fn}(key: &str) -> Option<&'static str> {{
    {table}
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| {table}[i].1)
}}"""


def emit_rust(
    printer_pages: dict[str, str],
    printer_subgroups: dict[str, str],
    printer_lines: dict[str, str],
    filament_pages: dict[str, str],
    filament_subgroups: dict[str, str],
    filament_lines: dict[str, str],
) -> str:
    tables = "\n\n".join(
        [
            _table("PRINTER_PAGES", printer_pages),
            _table("PRINTER_SUBGROUPS", printer_subgroups),
            _table("PRINTER_LINES", printer_lines),
            _table("FILAMENT_PAGES", filament_pages),
            _table("FILAMENT_SUBGROUPS", filament_subgroups),
            _table("FILAMENT_LINES", filament_lines),
        ]
    )
    fns = "\n\n".join(
        [
            _lookup_fn(
                "printer_page_of",
                "PRINTER_PAGES",
                "The printer-settings category the key appears under in Orca's "
                "`TabPrinter`\n/// (page for machine-wide keys, optgroup for "
                "per-extruder keys), or `None`.",
            ),
            _lookup_fn(
                "printer_subgroup_of",
                "PRINTER_SUBGROUPS",
                "The optgroup (sub-section within a page) a machine-wide option "
                "appears under,\n/// or `None`.",
            ),
            _lookup_fn(
                "printer_line_of",
                "PRINTER_LINES",
                "The label of the multi-option line a machine-wide key sits on "
                "(\"Resonance Avoidance\n/// Speed\", \"Frequency\", …), or "
                "`None`. Groups paired rows whose own labels\n/// are generic "
                "(\"Min\"/\"Max\", \"X\"/\"Y\") under one header.",
            ),
            _lookup_fn(
                "filament_page_of",
                "FILAMENT_PAGES",
                "The filament-settings page the key appears under in Orca's "
                "`TabFilament`\n/// (Filament, Print temperature, Cooling, …), "
                "or `None` for keys not laid out\n/// there (metadata, "
                "internal). This is the filament editor's visibility signal.",
            ),
            _lookup_fn(
                "filament_subgroup_of",
                "FILAMENT_SUBGROUPS",
                "The optgroup within a filament page (e.g. \"Basic information\" "
                "under the\n/// \"Filament\" page), or `None`.",
            ),
            _lookup_fn(
                "filament_line_of",
                "FILAMENT_LINES",
                "The label of the multi-option line a filament key sits on (the "
                "plate type\n/// for bed temps, \"Nozzle\" for print temps), or "
                "`None`. Disambiguates keys\n/// whose own label is generic "
                "(\"Other layers\" / \"First layer\").",
            ),
        ]
    )
    return f"""//! Per-key printer/filament-settings category, scraped from OrcaSlicer.
//!
//! AUTO-GENERATED — do not edit by hand. Regenerate with
//! `scripts/scrape_option_printer_pages.py` after pulling new upstream
//! OrcaSlicer source.
//!
//! Printer + filament options carry no libslic3r `category` of their own;
//! their grouping lives in `src/slic3r/GUI/Tab.cpp` (`TabPrinter` /
//! `TabFilament`). Each table maps a key to the `add_options_page` title it
//! appears under (and `new_optgroup` sub-title). Keys absent from a table
//! return `None`; callers fall back to an "Other" bucket. `printer_page_of`
//! / `filament_page_of` being `Some` is also the "Orca lays out an editor
//! for this key" signal the machine + filament panels gate visibility on.
//!
//! The per-extruder membership (options sized to the extruder count) is not
//! scraped here — it comes off the FFI `OptionDef::per_extruder` field, read
//! straight from libslic3r's `extruder_option_keys()`.

{tables}

{fns}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn tables_are_sorted_for_binary_search() {{
        for table in [PRINTER_PAGES, PRINTER_SUBGROUPS, PRINTER_LINES, FILAMENT_PAGES, FILAMENT_SUBGROUPS, FILAMENT_LINES] {{
            let mut last = "";
            for (key, _) in table {{
                assert!(*key > last, "table must be sorted; {{key}} <= {{last}}");
                last = key;
            }}
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
        // Paired rows carry the multi-option line label that gives their
        // generic "Min"/"Max" own-labels meaning.
        assert_eq!(
            printer_line_of("min_resonance_avoidance_speed"),
            Some("Resonance Avoidance Speed"),
        );
        assert_eq!(printer_line_of("input_shaping_freq_x"), Some("Frequency"));
        // A self-labeled single-option line has no line label.
        assert_eq!(printer_line_of("gcode_flavor"), None);
    }}

    #[test]
    fn filament_keys_map_to_their_orca_page() {{
        // Page = the add_options_page title; subgroup = the optgroup.
        assert_eq!(filament_page_of("nozzle_temperature"), Some("Filament"));
        assert_eq!(filament_subgroup_of("nozzle_temperature"), Some("Print temperature"));
        assert_eq!(filament_page_of("filament_type"), Some("Filament"));
        assert_eq!(filament_subgroup_of("filament_type"), Some("Basic information"));
        assert_eq!(filament_page_of("fan_max_speed"), Some("Cooling"));
        // Process / printer keys are not in the filament tables.
        assert!(filament_page_of("gcode_flavor").is_none());
        assert!(filament_page_of("layer_height").is_none());
    }}

    #[test]
    fn bed_temp_keys_carry_their_plate_line_label() {{
        // The plate type is the multi-option line label; the key's own label
        // is just "Other layers" / "First layer".
        assert_eq!(filament_line_of("cool_plate_temp"), Some("Cool Plate"));
        assert_eq!(filament_line_of("textured_plate_temp"), Some("Textured PEI Plate"));
        assert_eq!(filament_line_of("nozzle_temperature"), Some("Nozzle"));
        // A self-labeled single-option line has no line label.
        assert!(filament_line_of("filament_type").is_none());
    }}

    #[test]
    fn unknown_key_returns_none() {{
        assert!(printer_page_of("totally_made_up_option").is_none());
        assert!(filament_page_of("totally_made_up_option").is_none());
    }}
}}
"""


def main() -> None:
    tab_text = TAB_CPP.read_text()
    printer_pages, printer_subgroups, printer_lines = scrape(
        tab_text, "Printer", extruder_aware=True
    )
    filament_pages, filament_subgroups, filament_lines = scrape(
        tab_text, "Filament", extruder_aware=False
    )
    if not printer_pages:
        raise SystemExit(
            f"error: no TabPrinter option keys found in {TAB_CPP.relative_to(REPO_ROOT)}"
        )
    if not filament_pages:
        raise SystemExit(
            f"error: no TabFilament option keys found in {TAB_CPP.relative_to(REPO_ROOT)}"
        )
    OUTPUT_PATH.write_text(
        emit_rust(
            printer_pages,
            printer_subgroups,
            printer_lines,
            filament_pages,
            filament_subgroups,
            filament_lines,
        )
    )
    print(
        f"wrote {OUTPUT_PATH.relative_to(REPO_ROOT)}: "
        f"{len(printer_pages)} printer keys, {len(filament_pages)} filament keys "
        f"({len(filament_lines)} with line labels)"
    )


if __name__ == "__main__":
    main()
