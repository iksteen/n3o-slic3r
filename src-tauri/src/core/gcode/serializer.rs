//! G-code serializer (PR-3-7).
//!
//! Inverse of PR-3-6's parser: given a slice of typed `Line` values
//! that came from the parser, emit G-code byte-for-byte identical
//! to what the parser was fed. The round-trip equality is the
//! project's **independent oracle** for the slice loop (Execution
//! Plan §5 exit criteria); we use it instead of needing a reference
//! slicer.
//!
//! The implementation is intentionally trivial: each variant
//! carries its original source bytes (`raw`) and line ending
//! verbatim, and the serializer emits `raw + line_ending` per line.
//! The typed fields on `Move` / `Comment` / `ToolChange` are
//! inspection-only — they don't drive emission.
//!
//! `LayerChange` is the one variant the serializer drops on the
//! floor:
//!
//! - Marker-detected `LayerChange` was synthesized adjacent to a
//!   `Comment` (the `;LAYER:<n>` marker), and that comment carries
//!   the source bytes for emission. Emitting the LayerChange too
//!   would double-emit the marker.
//! - Heuristic-detected `LayerChange` has no source bytes — it
//!   exists for inspection only. Emitting it would *add* a marker
//!   the source didn't have.
//!
//! Either way, skipping LayerChange is the correct behavior.
//!
//! Future-self note: when Phase 8 plugins mutate typed fields on a
//! `Move`, the mutated line's `raw` is stale. Plugins are
//! responsible for invalidating or rewriting `raw` themselves;
//! the serializer doesn't try to re-derive. This is a Phase 8
//! responsibility — for Phase 3, every Move comes straight from
//! the parser so `raw` is always current.

use std::io::{self, Write};

use super::model::Line;

/// Write a sequence of `Line` values to `out`. Each variant emits
/// `<raw bytes> + <line ending>`, except `LayerChange` which is
/// elided (see module docs). The serializer makes no allocations
/// beyond what `write!` requires.
pub fn write_lines<W: Write>(lines: &[Line], mut out: W) -> io::Result<()> {
    for line in lines {
        write_line(line, &mut out)?;
    }
    Ok(())
}

/// Convenience wrapper that materializes the output into a `String`.
/// Used by tests; production callers prefer `write_lines` against a
/// `BufWriter` so they don't allocate the whole thing.
pub fn to_string(lines: &[Line]) -> String {
    let mut buf = Vec::new();
    // Writing to a Vec<u8> is infallible — unwrap is safe.
    write_lines(lines, &mut buf).expect("Vec<u8> write is infallible");
    String::from_utf8(buf).expect("parser preserves UTF-8 source bytes")
}

fn write_line<W: Write>(line: &Line, out: &mut W) -> io::Result<()> {
    match line {
        Line::Move(m) => {
            out.write_all(m.raw.as_bytes())?;
            out.write_all(m.line_ending.as_bytes())?;
        }
        Line::Comment(c) => {
            out.write_all(c.raw.as_bytes())?;
            out.write_all(c.line_ending.as_bytes())?;
        }
        Line::ToolChange(t) => {
            out.write_all(t.raw.as_bytes())?;
            out.write_all(t.line_ending.as_bytes())?;
        }
        Line::Other(o) => {
            out.write_all(o.raw.as_bytes())?;
            out.write_all(o.line_ending.as_bytes())?;
        }
        Line::LayerChange(_) => {
            // Synthetic — see module docs. The adjacent Comment
            // (for marker-detected) or surrounding moves (for
            // heuristic-detected) already carry the source bytes.
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::parser::parse_str;

    /// The whole point of the serializer: parse → serialize is the
    /// identity on the byte level.
    fn assert_round_trip(src: &str) {
        let lines = parse_str(src);
        let out = to_string(&lines);
        assert_eq!(out, src, "byte round-trip failed:\n--- source ---\n{src}\n--- emitted ---\n{out}\n");
    }

    #[test]
    fn round_trip_basic_move() {
        assert_round_trip("G1 X10 Y20 E0.5 F1200\n");
    }

    #[test]
    fn round_trip_preserves_parameter_order() {
        // F-first and F-last produce identical bytes through
        // parse → serialize. param_order on the Move is captured
        // for inspection; emission reads from `raw` which preserves
        // the actual source ordering regardless.
        assert_round_trip("G1 F100 X10 Y10\n");
        assert_round_trip("G1 X10 Y10 F100\n");
    }

    #[test]
    fn round_trip_preserves_zero_padded_command() {
        assert_round_trip("G01 X10\n");
        assert_round_trip("G00 X10 Y10\n");
    }

    #[test]
    fn round_trip_indented_semicolon_comment() {
        assert_round_trip("  ; hello world\n");
    }

    #[test]
    fn round_trip_paren_comment() {
        assert_round_trip("(setup)\n");
    }

    #[test]
    fn round_trip_inline_comment_on_move() {
        assert_round_trip("G1 X10 ; final move\n");
    }

    #[test]
    fn round_trip_tool_change_with_leading_whitespace() {
        assert_round_trip("  T2\n");
    }

    #[test]
    fn round_trip_mixed_line_endings() {
        let src = "G1 X1\nG1 X2\r\nG1 X3\n";
        assert_round_trip(src);
    }

    #[test]
    fn round_trip_unknown_command_via_other() {
        assert_round_trip("M104 S210\n");
        assert_round_trip("M140 S60\n");
    }

    #[test]
    fn round_trip_layer_marker_is_idempotent() {
        // The parser synthesizes a LayerChange after a `;LAYER:`
        // comment for inspection purposes. The serializer drops the
        // synthetic and re-emits only the comment — so the byte
        // output matches the source. Without this elision, the
        // marker would double-emit.
        let src = ";LAYER:5\nG1 X1\n";
        assert_round_trip(src);
    }

    #[test]
    fn round_trip_full_synthetic_block() {
        let src = "\
; estimated printing time = 1h 23m
; filament used [g] = 4.2
M104 S210
M140 S60
G28
;LAYER:0
;Z:0.2
;TYPE:Perimeter
G1 F1200 X10.000 Y10.000 E0.0200
G1 X20.000 Y10.000 E0.0400
;TYPE:Internal infill
G1 X20.000 Y20.000 E0.0600
T0
;LAYER:1
G1 X10.000 Y10.000 E0.0800
";
        assert_round_trip(src);
    }

    #[test]
    fn round_trip_via_writer_matches_to_string() {
        let src = "G1 X1\nG1 X2\n";
        let lines = parse_str(src);
        let mut buf = Vec::new();
        write_lines(&lines, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert_eq!(out, src);
        assert_eq!(out, to_string(&lines));
    }

    #[test]
    fn round_trip_empty_input() {
        assert_round_trip("");
    }

    #[test]
    fn round_trip_input_without_trailing_newline() {
        // Last line has no line ending — parser preserves that as
        // `line_ending = ""`. Serializer must too.
        assert_round_trip("G1 X1\nG1 X2");
    }
}
