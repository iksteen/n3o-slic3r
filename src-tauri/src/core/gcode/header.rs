//! Multi-slicer header metadata parser.
//!
//! Reads the comment block at the top of a G-code file and pulls
//! out typed estimates (time, filament use, layer count, slicer of
//! origin, bbox) regardless of which slicer authored the file —
//! Orca, Bambu Studio, PrusaSlicer, Cura.
//!
//! Phase 6's preview is the primary consumer (FR-GP-11): drag a
//! foreign `.gcode` onto the viewport, see the stats panel populate
//! immediately without needing to re-slice.
//!
//! The slice/summary module owns the libslic3r-specific scan that
//! builds a `PlateSummary` from just-emitted slice output. This
//! module overlaps a bit and is deliberately the more lenient
//! generalization: every field is best-effort, no failures, all
//! unknowns preserved in `raw_settings` so plugins can reach them
//! by key.

use std::collections::BTreeMap;
use std::io::BufRead;

use serde::{Deserialize, Serialize};

/// Best-effort parse of a G-code header. Every field is
/// `Option<...>` or empty-by-default — foreign G-code may have
/// any subset.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HeaderMetadata {
    pub slicer: Option<SlicerOrigin>,
    pub slicer_version: Option<String>,
    /// Estimated print time as a free-form string. The format
    /// varies wildly per slicer (`"1h 23m"`, `"83m 12s"`,
    /// `"4992.3"` seconds). Phase 6's preview parses this into a
    /// `Duration` for display; we don't here so we don't have to
    /// guess wrong.
    pub estimated_time: Option<String>,
    /// One entry per recognized "filament used" line. The unit is
    /// captured separately. PrusaSlicer emits `filament used [g]`,
    /// `[mm]`, `[cm3]` as separate lines for the same print.
    pub filament_used: Vec<FilamentUsage>,
    pub layer_count: Option<u32>,
    pub object_count: Option<u32>,
    pub bbox_min: Option<[f32; 3]>,
    pub bbox_max: Option<[f32; 3]>,
    /// Every `; key = value` line we recognized as such but didn't
    /// have a typed slot for. Plugins reach in by key. Cleared of
    /// the keys that are extracted into typed fields above so this
    /// stays a "what else" map.
    pub raw_settings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlicerOrigin {
    Orca,
    BambuStudio,
    PrusaSlicer,
    Cura,
    /// Caught a generator line we didn't recognize. The raw text
    /// is preserved so the UI can surface it as-is.
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilamentUsage {
    /// `g`, `mm`, `cm3`, `m`, `mm3`, etc. — whatever the slicer
    /// wrote in the `[..]` qualifier.
    pub unit: String,
    /// Raw value string. Often comma-separated for multi-extruder
    /// (`"4.2, 3.1, 0.0, 0.0"`). Phase 6's preview owns the
    /// per-extruder split.
    pub value: String,
}

const MAX_HEADER_LINES: usize = 4096;

/// Read up to `MAX_HEADER_LINES` lines from `input` and dispatch
/// comment patterns to typed fields. Reading stops at the first
/// non-comment line or EOF, whichever comes first — production
/// G-code emits the entire header in a contiguous comment block.
///
/// Use this for foreign-gcode header scanning where you want a
/// bounded read on possibly-pathological input (Phase 6 preview
/// drag-drop). For libslic3r-output where the slice-summary
/// metadata lives in the trailing CONFIG_BLOCK, the caller must
/// pre-collect comments + use [`parse_all_metadata`] instead — see
/// `core::slice::summary::build_summary` for that path.
pub fn parse_header<R: BufRead>(input: R) -> HeaderMetadata {
    let mut meta = HeaderMetadata::default();
    let mut count = 0;
    for line_result in input.lines() {
        count += 1;
        if count > MAX_HEADER_LINES {
            break;
        }
        let Ok(line) = line_result else {
            break;
        };
        let trimmed = line.trim();
        // Stop at the first non-comment, non-blank line — the
        // header is the prelude block.
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.starts_with(';') && !trimmed.starts_with('(') {
            break;
        }
        dispatch_line(trimmed, &mut meta);
    }
    meta
}

/// Convenience wrapper for in-memory strings.
pub fn parse_header_str(src: &str) -> HeaderMetadata {
    parse_header(src.as_bytes())
}

/// Walk every comment-prefixed line in `input` (no line cap, no
/// early bail on non-comment lines) and dispatch the typed fields.
/// Built for the libslic3r-output summary path: the slice-summary
/// metadata (`total layers count`, `estimated printing time`,
/// `filament used`) lives in the trailing CONFIG_BLOCK that
/// [`parse_header`] never reaches.
///
/// Caller is expected to have pre-filtered to comment lines (so
/// non-comments are tolerated as a no-op rather than terminating
/// the walk) — see `core::slice::summary::collect_comment_lines`.
pub fn parse_all_metadata<R: BufRead>(input: R) -> HeaderMetadata {
    let mut meta = HeaderMetadata::default();
    for line_result in input.lines() {
        let Ok(line) = line_result else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.starts_with(';') && !trimmed.starts_with('(') {
            // Non-comment lines are tolerated (e.g. interleaved
            // gcode commands the caller forgot to filter) — just
            // skip and keep walking.
            continue;
        }
        dispatch_line(trimmed, &mut meta);
    }
    meta
}

/// Strip the leading `;` / `(`-`)` delimiter from a comment-line
/// and pass the inner content to [`dispatch`]. Shared by
/// [`parse_header`] and [`parse_all_metadata`].
fn dispatch_line(trimmed: &str, meta: &mut HeaderMetadata) {
    let content = if let Some(rest) = trimmed.strip_prefix(';') {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix('(') {
        rest.trim_end_matches(')').trim()
    } else {
        trimmed
    };
    dispatch(content, meta);
}

fn dispatch(content: &str, meta: &mut HeaderMetadata) {
    // Generator / slicer identity. Three variants in the wild:
    //   `; generated by OrcaSlicer 2.3.0 on …` (Orca / Bambu / Prusa)
    //   `;Generated with Ultimaker Cura SteamEngine 5.6.0`     (Cura)
    //   `;Generator: …`                                         (rare)
    for prefix in ["generated by ", "generated with ", "generator:"] {
        if let Some(rest) = strip_prefix_ci(content.trim_start(), prefix) {
            meta.slicer = Some(detect_origin(rest));
            meta.slicer_version = extract_version(rest);
            return;
        }
    }

    // `printer_model = MK3S` (PrusaSlicer)
    if let Some(rest) = strip_after_eq(content, "printer_model") {
        meta.raw_settings
            .insert("printer_model".into(), rest.to_owned());
        return;
    }

    // `; estimated printing time (normal mode) = …` and the
    // simpler `; estimated printing time = …`.
    if let Some(rest) = strip_after_eq(content, "estimated printing time") {
        meta.estimated_time = Some(rest.to_owned());
        return;
    }
    // BambuStudio (vendored cascade output): `; model printing
    // time: 13m 6s; total estimated time: 19m 2s` — two stats on
    // one line, colon separator instead of `=`. We take the
    // *total* estimated time (includes start/end macros + heat-up)
    // because that's what the UI's "ETA" should show.
    if let Some(rest) = strip_prefix_ci(content, "model printing time:") {
        // Pull "total estimated time:" out of the same line if
        // present; otherwise fall back to the model time we just
        // matched.
        if let Some(total_after) = rest.find("total estimated time:") {
            let after = &rest[total_after + "total estimated time:".len()..];
            meta.estimated_time = Some(after.trim().trim_end_matches(';').trim().to_owned());
        } else {
            meta.estimated_time = Some(rest.trim().trim_end_matches(';').trim().to_owned());
        }
        return;
    }
    // Cura: `;TIME:4992` — total time in seconds, no `=`.
    if let Some(rest) = strip_prefix_ci(content, "time:") {
        meta.estimated_time = Some(rest.trim().to_owned());
        return;
    }

    // `; filament used [g] = 4.2, 3.1`. The PrusaSlicer/Orca form
    // includes a units qualifier in brackets; we capture it
    // separately so callers can pick the unit they want.
    if let Some(rest) = strip_prefix_ci(content, "filament used") {
        let rest = rest.trim_start();
        let (unit, value) = if let Some(after_bracket) = rest.strip_prefix('[') {
            if let Some(end) = after_bracket.find(']') {
                let unit = after_bracket[..end].trim().to_owned();
                let after = after_bracket[end + 1..].trim_start();
                let value = after
                    .strip_prefix('=')
                    .map(|v| v.trim().to_owned())
                    .unwrap_or_else(|| after.to_owned());
                (unit, value)
            } else {
                ("".to_owned(), rest.to_owned())
            }
        } else {
            // No bracket — Cura form: `;Filament used: 4.2m`.
            let after = rest.strip_prefix(':').map(|v| v.trim()).unwrap_or(rest);
            ("".to_owned(), after.to_owned())
        };
        meta.filament_used.push(FilamentUsage { unit, value });
        return;
    }

    // `; total layers count = 247` (PrusaSlicer/Orca).
    // `;LAYER_COUNT:247` (Cura).
    // `; total layer number: 100` (BambuStudio — colon separator,
    // singular "layer number" not "layers count").
    if let Some(rest) = strip_after_eq(content, "total layers count") {
        if let Ok(n) = rest.trim().parse::<u32>() {
            meta.layer_count = Some(n);
            return;
        }
    }
    if let Some(rest) = strip_prefix_ci(content, "layer_count:") {
        if let Ok(n) = rest.trim().parse::<u32>() {
            meta.layer_count = Some(n);
            return;
        }
    }
    if let Some(rest) = strip_prefix_ci(content, "total layer number:") {
        if let Ok(n) = rest.trim().parse::<u32>() {
            meta.layer_count = Some(n);
            return;
        }
    }

    // `; number of objects = 3` (Orca/Bambu sometimes).
    if let Some(rest) = strip_after_eq(content, "number of objects") {
        if let Ok(n) = rest.trim().parse::<u32>() {
            meta.object_count = Some(n);
            return;
        }
    }

    // `; min_x = …` style — collect into bbox.
    for (key, axis) in [("min_x", 0usize), ("min_y", 1), ("min_z", 2)] {
        if let Some(rest) = strip_after_eq(content, key) {
            if let Ok(v) = rest.trim().parse::<f32>() {
                let arr = meta.bbox_min.get_or_insert([0.0_f32; 3]);
                arr[axis] = v;
                return;
            }
        }
    }
    for (key, axis) in [("max_x", 0usize), ("max_y", 1), ("max_z", 2)] {
        if let Some(rest) = strip_after_eq(content, key) {
            if let Ok(v) = rest.trim().parse::<f32>() {
                let arr = meta.bbox_max.get_or_insert([0.0_f32; 3]);
                arr[axis] = v;
                return;
            }
        }
    }

    // Generic `; key = value` capture. Useful for plugin authors
    // who want to read arbitrary slicer settings out of the header
    // without us knowing each one. Skip values that look like
    // multi-line continuations or already-handled keys.
    if let Some((key, value)) = split_kv(content) {
        meta.raw_settings.insert(key, value);
    }
}

/// Detect slicer origin from a `generated by` line value. Lowercased
/// substring match — covers PrusaSlicer's `PrusaSlicer-2.7.4`,
/// Orca's `OrcaSlicer 2.3.0`, Bambu's `BambuStudio-01.05.00.61`,
/// Cura's `Ultimaker Cura SteamEngine 5.6.0`.
fn detect_origin(text: &str) -> SlicerOrigin {
    let lower = text.to_ascii_lowercase();
    if lower.contains("orca") {
        SlicerOrigin::Orca
    } else if lower.contains("bambustudio") || lower.contains("bambu studio") {
        SlicerOrigin::BambuStudio
    } else if lower.contains("prusaslicer") || lower.contains("prusa slicer") {
        SlicerOrigin::PrusaSlicer
    } else if lower.contains("cura") {
        SlicerOrigin::Cura
    } else {
        SlicerOrigin::Unknown(text.trim().to_owned())
    }
}

/// Pull a version-looking token out of a generator line. Looks for
/// the first sequence of `<digits>.<digits>` (optionally with
/// further `.X` components and trailing pre-release tags). Returns
/// `None` if nothing matched.
fn extract_version(text: &str) -> Option<String> {
    let mut buf = String::new();
    let mut chars = text.chars().peekable();
    let mut started = false;
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            buf.push(c);
            started = true;
        } else if c == '.' && started {
            // Lookahead: must be followed by another digit.
            if chars.peek().map(|n| n.is_ascii_digit()).unwrap_or(false) {
                buf.push(c);
            } else {
                break;
            }
        } else if started {
            // Allow `-` and alphanumeric in pre-release tags
            // (Orca's `2.3.0-rc`, BBS's `01.05.00.61` already
            // matched). Stop at whitespace.
            if c.is_whitespace() {
                break;
            }
            if c == '-' || c.is_alphanumeric() {
                buf.push(c);
            } else {
                break;
            }
        }
    }
    if buf.contains('.') {
        Some(buf)
    } else {
        None
    }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    // `s.get(..n)` returns None if `n` doesn't fall on a UTF-8 char
    // boundary — important for Snapmaker's machine_start_gcode which
    // carries CJK comments like `===== 床面异物检测 =====` that the
    // byte-indexed slice would panic on.
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Strip the `key` prefix (case-insensitive), then the `=` separator
/// (allowing whitespace around it), and return the trimmed value
/// side. Returns `None` when the line doesn't match the `key = …`
/// pattern.
fn strip_after_eq<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    let trimmed = s.trim_start();
    let head = trimmed.get(..key.len())?;
    if !head.eq_ignore_ascii_case(key) {
        return None;
    }
    let after = trimmed[key.len()..].trim_start();
    // Allow `key [units]` modifier (PrusaSlicer/Orca's `filament
    // used [g] = …`) or `key (qualifier)` (Orca's `estimated
    // printing time (normal mode) = …`).
    let after = if let Some(after_bracket) = after.strip_prefix('[') {
        match after_bracket.find(']') {
            Some(end) => after_bracket[end + 1..].trim_start(),
            None => after,
        }
    } else if let Some(after_paren) = after.strip_prefix('(') {
        match after_paren.find(')') {
            Some(end) => after_paren[end + 1..].trim_start(),
            None => after,
        }
    } else {
        after
    };
    after.strip_prefix('=').map(|v| v.trim())
}

/// Split a `; key = value` style comment body into `(key, value)`.
/// Conservative — only accepts ASCII alphanumeric / underscore /
/// dot / hyphen / square-bracket in the key so values like
/// `1.2 = 3.4` (a math-y comment) don't get misclassified.
fn split_kv(content: &str) -> Option<(String, String)> {
    let eq = content.find('=')?;
    let key = content[..eq].trim();
    if key.is_empty() {
        return None;
    }
    let key_ok = key.chars().all(|c| {
        c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '[' || c == ']' || c == ' '
    });
    if !key_ok {
        return None;
    }
    let value = content[eq + 1..].trim();
    Some((key.to_owned(), value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orca_header_extracts_typed_fields() {
        let src = "; generated by OrcaSlicer 2.3.0 on 2026-05-23\n\
; estimated printing time (normal mode) = 1h 23m 45s\n\
; filament used [g] = 4.21, 3.10\n\
; filament used [mm] = 1400.5, 1029.4\n\
; total layers count = 247\n\
; printer_model = A1mini\n\
M104 S210\n";
        let meta = parse_header_str(src);
        assert_eq!(meta.slicer, Some(SlicerOrigin::Orca));
        assert_eq!(meta.slicer_version.as_deref(), Some("2.3.0"));
        assert_eq!(
            meta.estimated_time.as_deref(),
            Some("1h 23m 45s"),
        );
        assert_eq!(meta.filament_used.len(), 2);
        assert_eq!(meta.filament_used[0].unit, "g");
        assert_eq!(meta.filament_used[0].value, "4.21, 3.10");
        assert_eq!(meta.filament_used[1].unit, "mm");
        assert_eq!(meta.layer_count, Some(247));
        assert_eq!(
            meta.raw_settings.get("printer_model").map(|s| s.as_str()),
            Some("A1mini"),
        );
    }

    #[test]
    fn prusaslicer_header_recognized() {
        let src = "; generated by PrusaSlicer 2.7.4+linux on 2026-05-22\n\
; estimated printing time = 1h 20m\n\
; filament used [cm3] = 3.21\n";
        let meta = parse_header_str(src);
        assert_eq!(meta.slicer, Some(SlicerOrigin::PrusaSlicer));
        assert!(meta
            .slicer_version
            .as_deref()
            .unwrap_or("")
            .starts_with("2.7.4"));
        assert_eq!(meta.filament_used[0].unit, "cm3");
    }

    #[test]
    fn bambu_studio_header_recognized() {
        let src = "; generated by BambuStudio-01.05.00.61 on 2026-05-22\n\
; estimated printing time = 45m\n";
        let meta = parse_header_str(src);
        assert_eq!(meta.slicer, Some(SlicerOrigin::BambuStudio));
        assert!(meta
            .slicer_version
            .as_deref()
            .unwrap_or("")
            .starts_with("01.05"));
    }

    #[test]
    fn cura_time_and_layer_count_recognized() {
        let src = ";Generated with Ultimaker Cura SteamEngine 5.6.0\n\
;TIME:4992\n\
;LAYER_COUNT:240\n\
;Filament used: 4.2m\n";
        let meta = parse_header_str(src);
        assert_eq!(meta.slicer, Some(SlicerOrigin::Cura));
        assert_eq!(meta.estimated_time.as_deref(), Some("4992"));
        assert_eq!(meta.layer_count, Some(240));
        assert!(!meta.filament_used.is_empty());
    }

    #[test]
    fn bbs_colon_separator_header_extracts_time_and_layer_count() {
        // Regression: BambuStudio-cascade output uses ':' separator
        // and singular "total layer number", not Orca's '=' +
        // "total layers count". Both formats must parse — without
        // this, phase3_smoke (which slices via the bundled cascade)
        // reports 0 layers and 0 time despite valid output.
        let src = "; HEADER_BLOCK_START\n\
                   ; generated by OrcaSlicer 2.4.0-dev on 2026-05-24\n\
                   ; model printing time: 13m 6s; total estimated time: 19m 2s\n\
                   ; total layer number: 100\n\
                   ; HEADER_BLOCK_END\n";
        let meta = parse_header_str(src);
        assert_eq!(meta.estimated_time.as_deref(), Some("19m 2s"));
        assert_eq!(meta.layer_count, Some(100));
    }

    #[test]
    fn empty_input_produces_default_metadata() {
        let meta = parse_header_str("");
        assert_eq!(meta, HeaderMetadata::default());
    }

    #[test]
    fn body_lines_stop_header_scan() {
        let src = "; estimated printing time = 1h\n\
G28\n\
; ignored = should-not-parse\n";
        let meta = parse_header_str(src);
        assert_eq!(meta.estimated_time.as_deref(), Some("1h"));
        // The comment after G28 is past the header block and should
        // not contaminate raw_settings.
        assert!(meta.raw_settings.get("ignored").is_none());
    }

    #[test]
    fn unknown_kv_lands_in_raw_settings() {
        let src = "; some_custom_key = some value\n";
        let meta = parse_header_str(src);
        assert_eq!(
            meta.raw_settings.get("some_custom_key").map(|s| s.as_str()),
            Some("some value"),
        );
    }

    #[test]
    fn detect_origin_handles_unknown_and_known() {
        assert_eq!(detect_origin("OrcaSlicer 2.3.0"), SlicerOrigin::Orca);
        assert_eq!(detect_origin("BambuStudio-01.05"), SlicerOrigin::BambuStudio);
        assert_eq!(detect_origin("PrusaSlicer-2.7.4"), SlicerOrigin::PrusaSlicer);
        assert_eq!(
            detect_origin("Cura SteamEngine"),
            SlicerOrigin::Cura,
        );
        assert!(matches!(
            detect_origin("Ye Olde Slicer 1.0"),
            SlicerOrigin::Unknown(_)
        ));
    }

    #[test]
    fn extract_version_handles_typical_strings() {
        assert_eq!(
            extract_version("OrcaSlicer 2.3.0 on 2026"),
            Some("2.3.0".into()),
        );
        assert_eq!(
            extract_version("BambuStudio-01.05.00.61"),
            Some("01.05.00.61".into()),
        );
        assert_eq!(
            extract_version("PrusaSlicer-2.7.4+linux"),
            Some("2.7.4".into()),
        );
        assert_eq!(extract_version("no version here"), None);
    }
}
