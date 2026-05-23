//! Post-slice summary extraction (PR-3-3 part 1).
//!
//! Reads the comment header libslic3r writes at the top of every
//! G-code file and surfaces the estimated time + filament use + layer
//! count + bounding box as a typed `PlateSummary`. The orchestrator
//! (PR-3-2) attaches this to every `slice:plate_finished` event so
//! the UI can render the summary card without re-parsing the file.
//!
//! Owns the FR-SL-4 deliverable on the libslic3r-output side. The
//! foreign-G-code header parser in `core::gcode::header` overlaps a
//! bit; we keep them separate because:
//!
//! - `core::gcode::header::parse_header` is lenient over any
//!   slicer's dialect and returns `Option<...>` for every field.
//!   Phase 6 preview reads this for drag-drop external files.
//! - `core::slice::summary::build_summary` is libslic3r-specific
//!   and produces a stricter `PlateSummary` shape the slice-finish
//!   event needs. It delegates to the lenient parser internally
//!   so we don't double-write the regex catalog.

use std::collections::BTreeMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::gcode::header::{parse_header, HeaderMetadata};

/// Typed result of scanning a libslic3r-emitted G-code header.
///
/// Every field falls back to a documented default on missing input
/// because libslic3r's emission schema varies a bit per version and
/// we'd rather surface "0g" than fail-the-whole-summary on one
/// missing line.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PlateSummary {
    /// Estimated print time in seconds. `0` if absent.
    pub estimated_time_seconds: u64,
    /// Raw "estimated printing time" string from the header,
    /// preserved for UI display. `"1h 23m"`, `"83m 12s"`, etc.
    pub estimated_time_text: String,
    /// Per-extruder filament use in grams. Zero-filled for
    /// extruders the header didn't report. Indexed by 0-based
    /// extruder slot — slot 0 is the first toolhead.
    pub filament_used_grams: BTreeMap<u8, f64>,
    /// Per-extruder filament use in millimeters of filament.
    pub filament_used_mm: BTreeMap<u8, f64>,
    /// Per-extruder filament use in cubic millimeters.
    pub filament_used_mm3: BTreeMap<u8, f64>,
    pub layer_count: u32,
    pub object_count: u32,
    /// `[min_x, min_y, min_z]` if reported; `None` otherwise.
    pub bbox_min: Option<[f32; 3]>,
    /// `[max_x, max_y, max_z]` if reported.
    pub bbox_max: Option<[f32; 3]>,
    /// Path of the G-code file the summary was scanned from.
    pub output_path: PathBuf,
}

/// Scan `gcode_path`'s header and produce a `PlateSummary`. Returns
/// `Err` only on I/O errors; missing fields in the header surface as
/// the variant's documented default, not as a failure.
pub fn build_summary(gcode_path: &Path) -> Result<PlateSummary, std::io::Error> {
    let file = std::fs::File::open(gcode_path)?;
    let header = parse_header(BufReader::new(file));
    Ok(summary_from_header(&header, gcode_path))
}

/// In-memory variant for tests + the future "summary from buffer"
/// path the orchestrator could take if libslic3r ever gives us the
/// G-code in memory (PRD §8.3's pending FFI extension).
pub fn build_summary_from_bytes(bytes: &[u8], gcode_path: &Path) -> PlateSummary {
    let header = parse_header(bytes);
    summary_from_header(&header, gcode_path)
}

fn summary_from_header(header: &HeaderMetadata, output_path: &Path) -> PlateSummary {
    let mut summary = PlateSummary {
        output_path: output_path.to_path_buf(),
        ..Default::default()
    };

    if let Some(text) = &header.estimated_time {
        summary.estimated_time_text = text.clone();
        summary.estimated_time_seconds = parse_duration_seconds(text).unwrap_or(0);
    }

    // libslic3r emits one `filament used` per unit it knows about
    // (`[g]`, `[mm]`, `[cm3]`). Values are typically comma-
    // separated per extruder.
    for usage in &header.filament_used {
        let per_extruder = parse_comma_floats(&usage.value);
        let target: &mut BTreeMap<u8, f64> = match usage.unit.trim() {
            "g" => &mut summary.filament_used_grams,
            "mm" => &mut summary.filament_used_mm,
            // Both `cm3` and `mm3` are reported by different slicers;
            // normalize to mm3 for our typed slot.
            "cm3" => {
                for (i, v) in per_extruder.into_iter().enumerate() {
                    summary
                        .filament_used_mm3
                        .insert(i as u8, v * 1000.0);
                }
                continue;
            }
            "mm3" => &mut summary.filament_used_mm3,
            _ => continue,
        };
        for (i, v) in per_extruder.into_iter().enumerate() {
            target.insert(i as u8, v);
        }
    }

    if let Some(n) = header.layer_count {
        summary.layer_count = n;
    }
    if let Some(n) = header.object_count {
        summary.object_count = n;
    }
    summary.bbox_min = header.bbox_min;
    summary.bbox_max = header.bbox_max;

    summary
}

/// Parse libslic3r's time-string flavors into a flat seconds count:
///
/// - `"1h 23m 45s"`
/// - `"83m 12s"`
/// - `"45s"`
/// - `"4992"` — Cura's bare-seconds form
/// - `"01h 23m 45s"` (zero-padded — same handling)
///
/// Returns `None` when nothing parseable was found.
fn parse_duration_seconds(text: &str) -> Option<u64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Bare integer / decimal — Cura's `;TIME:4992`.
    if let Ok(n) = trimmed.parse::<f64>() {
        return Some(n.round() as u64);
    }

    let mut total: u64 = 0;
    let mut buf = String::new();
    let mut any = false;
    for c in trimmed.chars() {
        if c.is_ascii_digit() || c == '.' {
            buf.push(c);
        } else if c.is_alphabetic() {
            if buf.is_empty() {
                continue;
            }
            let val: f64 = buf.parse().unwrap_or(0.0);
            let mult = match c.to_ascii_lowercase() {
                'd' => 86_400,
                'h' => 3_600,
                'm' => 60,
                's' => 1,
                _ => 0,
            };
            total += (val * mult as f64).round() as u64;
            buf.clear();
            any = true;
        } else if c.is_whitespace() {
            // Whitespace alone doesn't terminate the digits
            // (e.g. `1 h 23 m`) but we don't expect that — Orca
            // emits `1h 23m`. Treat it as boundary anyway so the
            // next token can start cleanly.
        }
    }
    if any {
        Some(total)
    } else {
        None
    }
}

/// Parse the typical `"4.21, 3.10, 0.0, 0.0"` per-extruder value
/// from a `filament used` line. Yields one f64 per extruder slot.
fn parse_comma_floats(value: &str) -> Vec<f64> {
    value
        .split(|c: char| c == ',' || c == ';')
        .map(|s| s.trim().parse::<f64>().unwrap_or(0.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tempfile(content: &str) -> PathBuf {
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        std::io::Write::write_all(&mut tmp, content.as_bytes()).unwrap();
        tmp.into_temp_path().keep().unwrap()
    }

    #[test]
    fn parses_orca_emitted_summary() {
        let src = "; generated by OrcaSlicer 2.3.0 on 2026-05-23\n\
; estimated printing time (normal mode) = 1h 23m 45s\n\
; filament used [g] = 4.21, 3.10\n\
; filament used [mm] = 1400.5, 1029.4\n\
; filament used [cm3] = 3.50, 2.58\n\
; total layers count = 247\n\
G28\n";
        let path = write_tempfile(src);
        let summary = build_summary(&path).expect("ok");
        assert_eq!(summary.estimated_time_seconds, 1 * 3600 + 23 * 60 + 45);
        assert_eq!(summary.estimated_time_text, "1h 23m 45s");
        assert_eq!(summary.filament_used_grams.get(&0), Some(&4.21));
        assert_eq!(summary.filament_used_grams.get(&1), Some(&3.10));
        assert_eq!(summary.filament_used_mm.get(&0), Some(&1400.5));
        // cm3 normalized to mm3 (×1000).
        assert_eq!(summary.filament_used_mm3.get(&0), Some(&3500.0));
        assert_eq!(summary.layer_count, 247);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parses_cura_emitted_summary() {
        let src = ";Generated with Ultimaker Cura SteamEngine 5.6.0\n\
;TIME:4992\n\
;LAYER_COUNT:240\n\
;Filament used: 4.2m\n\
G28\n";
        let path = write_tempfile(src);
        let summary = build_summary(&path).expect("ok");
        assert_eq!(summary.estimated_time_seconds, 4992);
        assert_eq!(summary.layer_count, 240);
        // Cura's filament-used line has no unit qualifier — it lands
        // in PR-3-8's header but our summary's typed slots stay
        // zero. Phase 6 may upgrade if Cura compatibility matters.
        assert!(summary.filament_used_grams.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_header_fields_fall_back_to_defaults() {
        let src = "; generated by OrcaSlicer 2.3.0\n\
G28\n\
G1 X0\n";
        let path = write_tempfile(src);
        let summary = build_summary(&path).expect("ok");
        assert_eq!(summary, PlateSummary {
            output_path: summary.output_path.clone(),
            ..PlateSummary::default()
        });
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn duration_parser_handles_canonical_forms() {
        assert_eq!(parse_duration_seconds("1h 23m 45s"), Some(5025));
        assert_eq!(parse_duration_seconds("83m 12s"), Some(83 * 60 + 12));
        assert_eq!(parse_duration_seconds("45s"), Some(45));
        assert_eq!(parse_duration_seconds("4992"), Some(4992));
        assert_eq!(parse_duration_seconds("4992.5"), Some(4993));
        assert_eq!(parse_duration_seconds(""), None);
        // Unrecognized format returns None.
        assert_eq!(parse_duration_seconds("yesterday"), None);
    }

    #[test]
    fn comma_floats_parses_typical_lists() {
        assert_eq!(parse_comma_floats("4.21, 3.10"), vec![4.21, 3.10]);
        assert_eq!(parse_comma_floats("4.21"), vec![4.21]);
        assert_eq!(parse_comma_floats(""), vec![0.0]);
        assert_eq!(parse_comma_floats("1.0, junk, 2.0"), vec![1.0, 0.0, 2.0]);
    }

    #[test]
    fn build_summary_from_bytes_does_not_touch_filesystem() {
        let src = "; estimated printing time = 30s\n; total layers count = 2\n";
        let summary = build_summary_from_bytes(src.as_bytes(), Path::new("(in-memory)"));
        assert_eq!(summary.estimated_time_seconds, 30);
        assert_eq!(summary.layer_count, 2);
        assert_eq!(summary.output_path, Path::new("(in-memory)"));
    }
}
