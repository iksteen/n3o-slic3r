//! Motion-only dry-run: neuter a G-code stream so the printer can
//! exercise the toolpath without heating up or extruding.
//!
//! Purpose: smoke test the send-and-print pipeline against real
//! hardware *before* committing the first full-temperature print. A
//! dry-run goes through homing, levels the bed, and traces every
//! XY motion the slice would have made — but with the hotend and
//! bed cold, and zero filament flow.
//!
//! Two transformations:
//!
//! 1. **Strip every E parameter** from G0/G1/G2/G3 motion lines
//!    (case-insensitive). The motion still happens, but no filament
//!    moves. `G1 X100 Y100 E5.2 F500` → `G1 X100 Y100 F500`.
//! 2. **Comment out heater commands** (M104/M109/M140/M190 — set/wait
//!    hotend / bed). Prefix the line with `; DRYRUN: ` so it's
//!    traceable in the printer's log but doesn't fire. `M109 S220`
//!    → `; DRYRUN: M109 S220`.
//!
//! Intentionally left alone:
//! - `G28` (home), `G29` (level), `G92 E0` (extruder counter reset)
//!   — needed for the head to land somewhere safe and for the
//!   stripped E never to accumulate.
//! - `M106`/`M107` (fan on/off) — fans don't hurt anything during
//!   a cold dry-run.
//! - All other M-codes (`M204` accel, `M220` feed override, etc.)
//!   — they shape the motion that the dry-run is precisely there
//!   to validate.
//! - Comments and empty lines — preserved byte-for-byte.

use std::io::{Cursor, Read, Write};

use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

use crate::core::threemf::md5_hex;

const HEATER_CODES: [&str; 4] = ["M104", "M109", "M140", "M190"];
const MOTION_CODES: [&str; 4] = ["G0", "G1", "G2", "G3"];

/// Convert a raw G-code stream into its motion-only dry-run form.
///
/// Round-trips line endings (`\n` and `\r\n` preserved per input
/// line). The output is guaranteed to be the same number of lines
/// as the input — heater commands are commented out, never deleted,
/// so a side-by-side diff lines up.
pub fn neuter_gcode(input: &str) -> String {
    // Pre-size: motion-line transforms grow when E is stripped; the
    // ": DRYRUN: " prefix on heaters is short. 1.1× the input
    // covers the common shape without a realloc.
    let mut out = String::with_capacity(input.len() + input.len() / 10);
    for line in input.split_inclusive('\n') {
        let (body, line_ending) = split_line_ending(line);
        let neutered = neuter_one_line(body);
        out.push_str(&neutered);
        out.push_str(line_ending);
    }
    out
}

fn split_line_ending(line: &str) -> (&str, &str) {
    if let Some(stripped) = line.strip_suffix("\r\n") {
        (stripped, "\r\n")
    } else if let Some(stripped) = line.strip_suffix('\n') {
        (stripped, "\n")
    } else {
        (line, "")
    }
}

fn neuter_one_line(line: &str) -> String {
    // Split off any inline comment. Heaters get the comment prefix
    // applied to the WHOLE original line so the comment is preserved
    // as-is on the same source line.
    let (code_part, comment_part) = match line.find(';') {
        Some(i) => (&line[..i], &line[i..]),
        None => (line, ""),
    };
    let trimmed = code_part.trim_start();
    if trimmed.is_empty() {
        return line.to_owned();
    }

    if is_heater_command(trimmed) {
        // The whole original line is a heater command — prefix-comment
        // it. Keeping the trailing inline comment (if any) intact so
        // the printer's log shows what the slicer intended.
        return format!("; DRYRUN: {line}");
    }

    if is_motion_command(trimmed) {
        let stripped = strip_e_parameter(code_part);
        // Re-attach the comment if present.
        if comment_part.is_empty() {
            stripped
        } else {
            format!("{stripped}{comment_part}")
        }
    } else {
        line.to_owned()
    }
}

fn first_token_matches(s: &str, candidates: &[&str]) -> bool {
    let end = s
        .find(|c: char| c.is_whitespace())
        .unwrap_or(s.len());
    let first = &s[..end];
    candidates
        .iter()
        .any(|c| first.eq_ignore_ascii_case(c))
}

fn is_heater_command(trimmed: &str) -> bool {
    first_token_matches(trimmed, &HEATER_CODES)
}

fn is_motion_command(trimmed: &str) -> bool {
    first_token_matches(trimmed, &MOTION_CODES)
}

/// Remove the `E<value>` parameter from a motion line. Preserves
/// whitespace around remaining tokens so byte-equivalent round-trip
/// is close enough for diff inspection. Handles upper- and lower-
/// case `E` and any numeric form (`E5`, `E5.2`, `E-0.5`, `E.5`).
fn strip_e_parameter(code_part: &str) -> String {
    let mut out = String::with_capacity(code_part.len());
    let mut chars = code_part.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if (c == 'E' || c == 'e')
            && i > 0
            && code_part[..i]
                .chars()
                .last()
                .map(|p| p.is_whitespace())
                .unwrap_or(false)
        {
            // Peek the next non-E character: must be a digit, minus
            // sign, or dot to qualify as an E parameter. Otherwise
            // it's e.g. "EXTRUDE" in a custom M-code argument, which
            // we leave alone.
            let next = chars.peek().map(|(_, ch)| *ch);
            if matches!(next, Some(ch) if ch == '-' || ch == '.' || ch.is_ascii_digit()) {
                // Pop the E value: optional sign, digits, optional
                // decimal, more digits.
                while let Some((_, nc)) = chars.peek() {
                    if nc.is_ascii_digit() || *nc == '-' || *nc == '.' {
                        chars.next();
                    } else {
                        break;
                    }
                }
                // Collapse the now-doubled space (we left the space
                // before `E` in `out`; the space AFTER E's value is
                // still in the input). Pop trailing space too so the
                // double doesn't become triple later.
                if let Some((_, ws)) = chars.peek() {
                    if ws.is_whitespace() {
                        chars.next();
                    }
                }
                // Trim a trailing space we may have left at end of
                // line if E was the last token.
                while out.ends_with(' ') {
                    out.pop();
                    if !out.ends_with(' ') {
                        out.push(' '); // Keep one space between tokens.
                        break;
                    }
                }
                if out.ends_with(' ')
                    && chars.peek().map(|(_, c)| c.is_whitespace()).unwrap_or(true)
                {
                    // E was last on the line — drop the trailing space.
                    out.pop();
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Neuter every `plate_<N>.gcode` entry inside a Bambu `.gcode.3mf`
/// bundle. The MD5 sidecar (`plate_<N>.gcode.md5`) is recomputed so
/// the printer's integrity check passes against the rewritten body.
/// All other entries (3dmodel.model, plate metadata, thumbnails) are
/// copied verbatim.
///
/// Returns a new bundle ready to hand to the driver's send path.
pub fn neuter_gcode_3mf(bundle: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let cursor = Cursor::new(bundle);
    let mut zip = ZipArchive::new(cursor)
        .map_err(|e| std::io::Error::other(format!("open .gcode.3mf: {e}")))?;

    let mut out_buf = Vec::with_capacity(bundle.len());
    {
        let out_cursor = Cursor::new(&mut out_buf);
        let mut writer = ZipWriter::new(out_cursor);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        // Two-pass: first collect every neutered plate gcode + its
        // updated md5 into a side map, then stream every original
        // entry into the writer, swapping in the neutered bytes when
        // we hit a plate file. Two-pass because we can't borrow the
        // archive twice (once to find names, once to read bodies).
        let neutered_plates = collect_neutered_plates(&mut zip)?;

        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| {
                std::io::Error::other(format!("read entry {i}: {e}"))
            })?;
            let name = entry.name().to_owned();
            writer
                .start_file(&name, opts)
                .map_err(|e| std::io::Error::other(format!("start_file {name}: {e}")))?;
            if let Some(neutered) = neutered_plates.get(&name) {
                writer.write_all(neutered)?;
            } else {
                std::io::copy(&mut entry, &mut writer)?;
            }
        }
        writer
            .finish()
            .map_err(|e| std::io::Error::other(format!("finalize zip: {e}")))?;
    }
    Ok(out_buf)
}

fn collect_neutered_plates<R: std::io::Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
) -> Result<std::collections::HashMap<String, Vec<u8>>, std::io::Error> {
    use std::collections::HashMap;
    let mut out: HashMap<String, Vec<u8>> = HashMap::new();
    let entry_names: Vec<String> = zip.file_names().map(|s| s.to_owned()).collect();
    for name in &entry_names {
        if let Some(rest) = name.strip_prefix("Metadata/plate_") {
            let Some(num_str) = rest.strip_suffix(".gcode") else {
                continue;
            };
            // Skip the .md5 sidecar (handled below by lookup).
            if num_str.contains('.') {
                continue;
            }
            let mut entry = zip
                .by_name(name)
                .map_err(|e| std::io::Error::other(format!("open {name}: {e}")))?;
            let mut body = String::new();
            entry.read_to_string(&mut body)?;
            let neutered = neuter_gcode(&body);
            let new_md5 = md5_hex(neutered.as_bytes());
            out.insert(name.clone(), neutered.into_bytes());
            out.insert(format!("{name}.md5"), new_md5.into_bytes());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_e_from_extrusion_move() {
        let input = "G1 X100 Y100 E5.2 F500\n";
        let out = neuter_gcode(input);
        assert_eq!(out, "G1 X100 Y100 F500\n");
    }

    #[test]
    fn strips_e_when_e_is_last_token() {
        let input = "G1 E2.5\n";
        let out = neuter_gcode(input);
        // Pure-E line: motion command remains but with nothing to
        // do. Printer interprets as a no-op move.
        assert_eq!(out, "G1\n");
    }

    #[test]
    fn strips_negative_e_value_retraction() {
        let input = "G1 E-1.5 F1800\n";
        let out = neuter_gcode(input);
        assert_eq!(out, "G1 F1800\n");
    }

    #[test]
    fn strips_e_with_leading_decimal() {
        let input = "G1 X10 E.5\n";
        let out = neuter_gcode(input);
        assert_eq!(out, "G1 X10\n");
    }

    #[test]
    fn preserves_g0_travel_move_unchanged() {
        let input = "G0 X50 Y50 F9000\n";
        let out = neuter_gcode(input);
        // No E param in input → output identical.
        assert_eq!(out, input);
    }

    #[test]
    fn comments_out_hotend_set() {
        let input = "M104 S220\n";
        let out = neuter_gcode(input);
        assert_eq!(out, "; DRYRUN: M104 S220\n");
    }

    #[test]
    fn comments_out_hotend_wait() {
        let input = "M109 S220\n";
        let out = neuter_gcode(input);
        assert_eq!(out, "; DRYRUN: M109 S220\n");
    }

    #[test]
    fn comments_out_bed_commands() {
        let input = "M140 S65\nM190 S65\n";
        let out = neuter_gcode(input);
        assert_eq!(out, "; DRYRUN: M140 S65\n; DRYRUN: M190 S65\n");
    }

    #[test]
    fn preserves_homing_and_leveling() {
        // The whole point of dry-run is that the printer still
        // homes and levels — without those, the head doesn't know
        // where it is and the safety claim collapses.
        let input = "G28\nG29\n";
        let out = neuter_gcode(input);
        assert_eq!(out, input);
    }

    #[test]
    fn preserves_extruder_counter_reset() {
        // G92 E0 is bookkeeping, not extrusion. Leave it alone so
        // any code that reads absolute-E doesn't see drift.
        let input = "G92 E0\n";
        let out = neuter_gcode(input);
        assert_eq!(out, input);
    }

    #[test]
    fn preserves_fan_commands() {
        let input = "M106 S255\nM107\n";
        let out = neuter_gcode(input);
        assert_eq!(out, input);
    }

    #[test]
    fn preserves_comments_and_blank_lines() {
        let input = "; LAYER:0\n\n; outer wall\n";
        let out = neuter_gcode(input);
        assert_eq!(out, input);
    }

    #[test]
    fn preserves_inline_comment_after_motion() {
        let input = "G1 X100 Y100 E5.2 F500 ; outer wall\n";
        let out = neuter_gcode(input);
        // E is stripped; the inline comment lands on the same line.
        assert!(out.contains("; outer wall"), "lost inline comment: {out:?}");
        assert!(!out.contains("E5.2"), "E param survived: {out:?}");
        assert!(out.starts_with("G1 X100 Y100"));
    }

    #[test]
    fn preserves_inline_comment_on_heater() {
        let input = "M104 S220 ; set hotend\n";
        let out = neuter_gcode(input);
        assert_eq!(out, "; DRYRUN: M104 S220 ; set hotend\n");
    }

    #[test]
    fn preserves_crlf_line_endings() {
        let input = "G1 X10 E2\r\nM104 S220\r\n";
        let out = neuter_gcode(input);
        assert!(out.contains("\r\n"), "lost CRLF: {out:?}");
        assert!(out.contains("G1 X10"));
        assert!(out.contains("; DRYRUN: M104 S220"));
    }

    #[test]
    fn preserves_line_count() {
        // Heater commands must be COMMENTED OUT, not deleted —
        // a side-by-side diff is the audit trail for dry-run.
        let input = "G28\nM104 S220\nG1 X100 E5\nM109 S220\nG1 X200 E10\n";
        let out = neuter_gcode(input);
        assert_eq!(input.lines().count(), out.lines().count());
    }

    #[test]
    fn case_insensitive_e_parameter() {
        // Most slicers emit upper-case; some hand-rolled gcode uses
        // lower-case. Strip both.
        let input = "g1 x10 e5\n";
        let out = neuter_gcode(input);
        assert_eq!(out, "g1 x10\n");
    }

    #[test]
    fn case_insensitive_heater_commands() {
        let input = "m104 s220\n";
        let out = neuter_gcode(input);
        assert_eq!(out, "; DRYRUN: m104 s220\n");
    }

    #[test]
    fn does_not_strip_e_inside_other_tokens() {
        // The "E" in "EXACT" is not a parameter — only an E
        // preceded by whitespace and followed by a number qualifies.
        let input = "; LAYER_TYPE:EXTERNAL_PERIMETER\nG1 X10\n";
        let out = neuter_gcode(input);
        assert_eq!(out, input);
    }

    #[test]
    fn does_not_touch_non_motion_non_heater_m_codes() {
        // Acceleration, feed override, etc. — the dry-run is a
        // motion-fidelity test, so these must NOT be stripped.
        let input = "M204 P10000 R5000\nM220 S95\nM221 S100\n";
        let out = neuter_gcode(input);
        assert_eq!(out, input);
    }

    #[test]
    fn bundle_round_trip_neuters_gcode_and_updates_md5() {
        // Build a minimal .gcode.3mf with one plate, neuter, then
        // re-read and verify the gcode body lost its E values + the
        // md5 sidecar matches the new body.
        use crate::core::threemf::{fixture_input, md5_hex, write_sliced_3mf};

        let gcode = b"\
;LAYER:0
M104 S220
G28
G1 X100 Y100 E5.2 F500
G1 X200 Y100 E10.4
M104 S0
"
        .to_vec();

        let tmp = tempfile::NamedTempFile::with_suffix(".gcode.3mf").unwrap();
        let input = fixture_input(1, gcode);
        write_sliced_3mf(&input, tmp.path()).expect("write bundle");

        let original_bytes = std::fs::read(tmp.path()).expect("read bundle");
        let neutered = neuter_gcode_3mf(&original_bytes).expect("neuter bundle");
        assert!(!neutered.is_empty());

        // Re-open the neutered bundle and pull the gcode + md5 back out.
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(neutered)).expect("open");
        let mut neutered_gcode = String::new();
        zip.by_name("Metadata/plate_1.gcode")
            .expect("plate_1.gcode")
            .read_to_string(&mut neutered_gcode)
            .expect("read gcode");
        let mut sidecar = String::new();
        zip.by_name("Metadata/plate_1.gcode.md5")
            .expect("plate_1.gcode.md5")
            .read_to_string(&mut sidecar)
            .expect("read md5");

        assert!(neutered_gcode.contains("; DRYRUN: M104 S220"));
        assert!(!neutered_gcode.contains("E5.2"));
        assert!(!neutered_gcode.contains("E10.4"));
        assert!(neutered_gcode.contains("G28\n"));

        // The md5 sidecar must match the NEW body, not the original.
        let expected = md5_hex(neutered_gcode.as_bytes());
        assert_eq!(sidecar, expected, "md5 sidecar must match neutered body");
    }

    #[test]
    fn realistic_layer_excerpt() {
        // A small excerpt that resembles what libslic3r emits per
        // layer, plus a heater within. Smoke test that all the
        // pieces work together end-to-end.
        let input = "\
;LAYER:0
M104 S220
G28
G1 Z0.2 F600
G1 X10 Y10 F9000
G1 X90 Y10 E5.2 F500 ; outer wall
G1 X90 Y90 E10.4 F500
G1 E-1.5 F1800 ; retract
M104 S0
";
        let out = neuter_gcode(input);
        // Heaters commented:
        assert!(out.contains("; DRYRUN: M104 S220"));
        assert!(out.contains("; DRYRUN: M104 S0"));
        // E values gone from motion lines:
        assert!(!out.contains("E5.2"));
        assert!(!out.contains("E10.4"));
        assert!(!out.contains("E-1.5"));
        // Motion structure preserved:
        assert!(out.contains("G1 X90 Y10 F500 ; outer wall"));
        assert!(out.contains("G28\n"));
        // Line count matches:
        assert_eq!(input.lines().count(), out.lines().count());
    }
}
