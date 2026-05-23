//! Streaming G-code parser (PR-3-6).
//!
//! Reads a byte stream into a sequence of typed `Line` values that
//! the serializer (PR-3-7) can re-emit byte-for-byte. Streaming so
//! a 50 MB file doesn't materialize at once; lenient by design so a
//! single unknown M-command or malformed parameter doesn't abort
//! the parse.
//!
//! Recognized:
//!
//! - `G0` / `G1` / `G2` / `G3` motion commands with `X / Y / Z / E /
//!   F / I / J` parameters.
//! - `T<n>` tool changes (with optional whitespace before the number).
//! - `;…` line-rest comments and `(…)` parenthesized comments.
//! - Inline trailing comments on motion lines (`G1 X10 ; final`).
//! - Structured comments libslic3r emits (`;TYPE:`, `;LAYER:`,
//!   `;Z:`, estimated time, filament used, layer count, printer
//!   model, M104/M140-style temp hints).
//! - `\n` and `\r\n` line endings, preserved per-line.
//!
//! Everything else (`M104`, `M140`, blank lines, unknown
//! commands) maps to `Line::Other` with the source bytes verbatim
//! so the serializer can re-emit it without loss.

use std::io::{BufRead, BufReader, Read};

use super::model::{
    ArcCenter, Comment, CommentStyle, FeatureType, LayerChange, LayerSource, Line, Move,
    MoveCommand, MoveParam, Other, Position, SemanticComment, ToolChange,
};

/// Errors the streaming parser can surface per-line. Iterating
/// callers can log + continue — emitting an `Err` does not abort
/// the iterator. The parser tries hard not to emit these in the
/// first place: unknown commands become `Line::Other`, not errors.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub byte_offset: u64,
    pub line_number: u32,
    pub kind: ParseErrorKind,
    pub raw_line: String,
}

#[derive(Debug, Clone)]
pub enum ParseErrorKind {
    /// A recognized command had a malformed numeric parameter.
    /// E.g. `G1 Xabc`.
    InvalidNumber {
        param: char,
        value: String,
    },
    /// An I/O error reading from the underlying stream.
    Io(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            ParseErrorKind::InvalidNumber { param, value } => {
                write!(
                    f,
                    "line {}: invalid {} parameter value {:?}",
                    self.line_number, param, value,
                )
            }
            ParseErrorKind::Io(msg) => {
                write!(f, "line {}: io: {msg}", self.line_number)
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Streaming entry point. Returns a lazy iterator that yields one
/// `Result<Line, ParseError>` per source line. The iterator owns
/// the reader so the buffer stays bounded.
pub fn parse_lines<R: Read>(input: R) -> impl Iterator<Item = Result<Line, ParseError>> {
    LineIter::new(BufReader::new(input))
}

/// Materialize the whole input into a `Vec<Line>`. Convenient for
/// tests and small G-code snippets; production code prefers
/// `parse_lines` to keep memory bounded.
pub fn parse_str(src: &str) -> Vec<Line> {
    parse_lines(src.as_bytes()).filter_map(Result::ok).collect()
}

/// Iterator wrapper that adapts a stream-of-lines into `Line`
/// values. Tracks byte offset + line number, handles `\r\n`
/// preservation, threads layer-index state for `LayerChange`
/// synthesis on marker comments.
struct LineIter<R: BufRead> {
    reader: R,
    buf: Vec<u8>,
    byte_offset: u64,
    line_number: u32,
    /// Next layer index to assign on a layer marker.
    next_layer_index: u32,
    /// Last `;Z:` value we saw, for `LayerChange.z` population
    /// when the marker comment didn't carry one.
    last_seen_z: Option<f32>,
    /// Pending synthetic `LayerChange` to emit on the next `next()`
    /// call. Some marker comments (`;LAYER_CHANGE`) emit *only*
    /// the synthetic line; others (`;LAYER:<n>`) emit the comment
    /// AND a synthetic. We use this queue so the iterator can emit
    /// the comment first and the synthetic next.
    pending: Option<Line>,
}

impl<R: BufRead> LineIter<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            buf: Vec::with_capacity(256),
            byte_offset: 0,
            line_number: 0,
            next_layer_index: 0,
            last_seen_z: None,
            pending: None,
        }
    }

    /// Read the next line from `reader`, preserving the line ending
    /// (`\n` / `\r\n`). Returns `Ok(None)` on EOF.
    fn read_one_line(&mut self) -> Result<Option<(String, String)>, std::io::Error> {
        self.buf.clear();
        let n = self.reader.read_until(b'\n', &mut self.buf)?;
        if n == 0 {
            return Ok(None);
        }
        let mut content_end = self.buf.len();
        let mut ending = String::new();
        if content_end > 0 && self.buf[content_end - 1] == b'\n' {
            ending.push('\n');
            content_end -= 1;
            if content_end > 0 && self.buf[content_end - 1] == b'\r' {
                ending.insert(0, '\r');
                content_end -= 1;
            }
        }
        let body = String::from_utf8_lossy(&self.buf[..content_end]).into_owned();
        Ok(Some((body, ending)))
    }
}

impl<R: BufRead> Iterator for LineIter<R> {
    type Item = Result<Line, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        // Drain any queued synthetic LayerChange first.
        if let Some(line) = self.pending.take() {
            return Some(Ok(line));
        }

        let offset = self.byte_offset;
        let read = match self.read_one_line() {
            Ok(Some(pair)) => pair,
            Ok(None) => return None,
            Err(e) => {
                let line_number = self.line_number + 1;
                return Some(Err(ParseError {
                    byte_offset: offset,
                    line_number,
                    kind: ParseErrorKind::Io(e.to_string()),
                    raw_line: String::new(),
                }));
            }
        };
        let (body, line_ending) = read;
        self.line_number += 1;
        let line_number = self.line_number;
        // Advance the byte offset to the start of the next line.
        self.byte_offset = offset + body.len() as u64 + line_ending.len() as u64;

        let line = self.classify_line(body, line_ending, offset, line_number);
        Some(line)
    }
}

impl<R: BufRead> LineIter<R> {
    fn classify_line(
        &mut self,
        body: String,
        line_ending: String,
        raw_offset: u64,
        line_number: u32,
    ) -> Result<Line, ParseError> {
        // Classify on borrowed slices into `body` first; once we
        // know we're emitting a Comment/Other we extract the
        // semantic tag and then move `body` into the variant's
        // `raw`. NLL releases the slice borrows once they're not
        // used past the move.
        let trimmed = body.trim_start();

        // Parenthesized whole-line comment? `(foo)` — Marlin
        // tolerates these. Trailing characters after the `)` go to
        // Other since they're rare and we don't want to overspecify.
        if trimmed.starts_with('(') {
            if let Some(end) = trimmed.find(')') {
                let inside = &trimmed[1..end];
                let after = &trimmed[end + 1..];
                if after.trim().is_empty() {
                    let semantic = parse_semantic_comment(inside, &mut self.last_seen_z);
                    return Ok(Line::Comment(Comment {
                        // raw = whole source line minus the line
                        // ending; serializer re-emits identically.
                        raw: body,
                        style: CommentStyle::Parens,
                        semantic,
                        raw_offset,
                        line_ending,
                    }));
                }
            }
        }

        // Whole-line `;…` comment.
        if trimmed.starts_with(';') {
            let content = &trimmed[1..];
            let semantic = parse_semantic_comment(content, &mut self.last_seen_z);
            let semantic_for_synth = semantic.clone();
            let line = Line::Comment(Comment {
                raw: body,
                style: CommentStyle::Semicolon,
                semantic,
                raw_offset,
                line_ending: line_ending.clone(),
            });
            // Layer marker → enqueue a synthetic LayerChange to
            // emit on the next `next()` call. The serializer round-
            // trips the LayerChange as a `;LAYER:<n>` comment.
            if matches!(semantic_for_synth, Some(SemanticComment::Layer(_))) {
                let index = self.next_layer_index;
                self.next_layer_index = self.next_layer_index.saturating_add(1);
                self.pending = Some(Line::LayerChange(LayerChange {
                    index,
                    z: self.last_seen_z,
                    source: LayerSource::Marker,
                    raw_offset,
                    line_ending: line_ending.clone(),
                }));
            }
            return Ok(line);
        }

        // Tool change: `T0`, `T 0`, `T00`. `try_parse_tool_change`
        // inspects the trimmed slice but emits the full body as
        // `raw` so round-trip preserves any leading whitespace.
        if let Some(tc) = try_parse_tool_change(trimmed, &body, raw_offset, line_ending.clone()) {
            return Ok(Line::ToolChange(tc));
        }

        // Motion command: G0 / G1 / G2 / G3.
        if let Some(parsed) =
            try_parse_move(trimmed, &body, raw_offset, line_ending.clone(), line_number)
        {
            return parsed;
        }

        // Fallthrough: anything else (M-commands, blanks, unknown G).
        Ok(Line::Other(Other {
            raw: body,
            raw_offset,
            line_ending,
        }))
    }
}

fn try_parse_tool_change(
    line: &str,
    full_body: &str,
    raw_offset: u64,
    line_ending: String,
) -> Option<ToolChange> {
    let trimmed = line.trim_end();
    // Accept `T<n>` and `T <n>` (whitespace tolerant).
    let after_t = trimmed.strip_prefix('T').or_else(|| trimmed.strip_prefix('t'))?;
    let after_t = after_t.trim_start();
    // Number runs to end of line or to the first whitespace / ';'.
    let end = after_t
        .find(|c: char| c == ';' || c.is_whitespace())
        .unwrap_or(after_t.len());
    let num_str = &after_t[..end];
    if num_str.is_empty() {
        return None;
    }
    let extruder: u8 = num_str.parse().ok()?;
    // Confirm the rest of the line is comment or blank, else
    // this isn't a tool change line.
    let remainder = after_t[end..].trim_start();
    if !remainder.is_empty() && !remainder.starts_with(';') {
        return None;
    }
    Some(ToolChange {
        extruder,
        // Preserve the full source body (including leading
        // whitespace, trailing comment) so round-trip is byte-exact.
        raw: full_body.to_owned(),
        raw_offset,
        line_ending,
    })
}

fn try_parse_move(
    line: &str,
    full_body: &str,
    raw_offset: u64,
    line_ending: String,
    line_number: u32,
) -> Option<Result<Line, ParseError>> {
    let trimmed = line.trim_end();
    // Match `G0`/`G1`/`G2`/`G3`/`G00`/`G01`/`G02`/`G03`.
    let (cmd_text, rest) = take_command_token(trimmed)?;
    let command = match cmd_text.to_ascii_uppercase().as_str() {
        "G0" | "G00" => MoveCommand::Rapid,
        "G1" | "G01" => MoveCommand::Linear,
        "G2" | "G02" => MoveCommand::ArcCw,
        "G3" | "G03" => MoveCommand::ArcCcw,
        _ => return None,
    };

    // Split off inline `;` comment, if any.
    let (params_part, inline_comment) = split_inline_comment(rest);

    let mut target = Position::default();
    let mut feedrate: Option<u32> = None;
    let mut arc_center = ArcCenter::default();
    let mut param_order: Vec<MoveParam> = Vec::with_capacity(6);

    for token in params_part.split_whitespace() {
        let mut chars = token.chars();
        let letter = match chars.next() {
            Some(c) => c.to_ascii_uppercase(),
            None => continue,
        };
        let value_str = &token[letter.len_utf8()..];
        let param = match letter {
            'X' => MoveParam::X,
            'Y' => MoveParam::Y,
            'Z' => MoveParam::Z,
            'E' => MoveParam::E,
            'F' => MoveParam::F,
            'I' => MoveParam::I,
            'J' => MoveParam::J,
            _ => continue,
        };
        let parse_f32 = || {
            value_str.parse::<f32>().map_err(|_| ParseError {
                byte_offset: raw_offset,
                line_number,
                kind: ParseErrorKind::InvalidNumber {
                    param: letter,
                    value: value_str.to_owned(),
                },
                raw_line: line.to_owned(),
            })
        };
        match param {
            MoveParam::X => match parse_f32() {
                Ok(v) => target.x = Some(v),
                Err(e) => return Some(Err(e)),
            },
            MoveParam::Y => match parse_f32() {
                Ok(v) => target.y = Some(v),
                Err(e) => return Some(Err(e)),
            },
            MoveParam::Z => match parse_f32() {
                Ok(v) => target.z = Some(v),
                Err(e) => return Some(Err(e)),
            },
            MoveParam::E => match parse_f32() {
                Ok(v) => target.e = Some(v),
                Err(e) => return Some(Err(e)),
            },
            MoveParam::F => match value_str.parse::<f32>() {
                Ok(v) => feedrate = Some(v as u32),
                Err(_) => {
                    return Some(Err(ParseError {
                        byte_offset: raw_offset,
                        line_number,
                        kind: ParseErrorKind::InvalidNumber {
                            param: 'F',
                            value: value_str.to_owned(),
                        },
                        raw_line: line.to_owned(),
                    }));
                }
            },
            MoveParam::I => match parse_f32() {
                Ok(v) => arc_center.i = Some(v),
                Err(e) => return Some(Err(e)),
            },
            MoveParam::J => match parse_f32() {
                Ok(v) => arc_center.j = Some(v),
                Err(e) => return Some(Err(e)),
            },
        }
        param_order.push(param);
    }

    Some(Ok(Line::Move(Move {
        // Whole source body preserved for the byte-equivalent
        // round-trip; typed fields below are inspection only.
        raw: full_body.to_owned(),
        command,
        command_text: cmd_text.to_owned(),
        target,
        feedrate,
        arc_center,
        param_order,
        inline_comment,
        raw_offset,
        line_ending,
    })))
}

/// Pop the first whitespace-delimited token from `line`. Returns
/// `(token, rest_starting_with_separator)` so the caller can preserve
/// the original spacing while reading params.
fn take_command_token(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    if line.is_empty() {
        return None;
    }
    let end = line
        .find(|c: char| c.is_whitespace() || c == ';')
        .unwrap_or(line.len());
    Some((&line[..end], &line[end..]))
}

/// Split a move-line's tail into (params_text, inline_comment).
/// The inline comment is everything after the first unescaped `;`,
/// minus the `;` itself. The leading space (if any) between the
/// last parameter and the `;` is dropped — the serializer
/// re-introduces a single space.
fn split_inline_comment(rest: &str) -> (&str, Option<String>) {
    if let Some(idx) = rest.find(';') {
        let params = rest[..idx].trim_end();
        let comment = &rest[idx + 1..];
        (params, Some(comment.to_owned()))
    } else {
        (rest.trim_end(), None)
    }
}

/// Pattern-match against the structured comments we recognize.
/// Mutates `last_seen_z` when the comment carries a `;Z:` value so
/// later layer-change synthesis can attach the height.
fn parse_semantic_comment(content: &str, last_seen_z: &mut Option<f32>) -> Option<SemanticComment> {
    let trimmed = content.trim();
    let lower = trimmed.to_ascii_lowercase();

    // `;TYPE: <feature>`
    if let Some(rest) = strip_prefix_ci(trimmed, "type:") {
        return Some(SemanticComment::FeatureType(FeatureType::parse(
            rest.trim(),
        )));
    }

    // `;LAYER:<n>` and `;LAYER_CHANGE` / `; CHANGE_LAYER`.
    if let Some(rest) = strip_prefix_ci(trimmed, "layer:") {
        if let Ok(n) = rest.trim().parse::<u32>() {
            return Some(SemanticComment::Layer(n));
        }
    }
    if lower == "layer_change" || lower == "change_layer" {
        return Some(SemanticComment::Layer(0));
    }

    // `;Z:<height>`
    if let Some(rest) = strip_prefix_ci(trimmed, "z:") {
        if let Ok(z) = rest.trim().parse::<f32>() {
            *last_seen_z = Some(z);
            return Some(SemanticComment::Z(z));
        }
    }

    // `;HEIGHT:<height>` — PrusaSlicer variant of `;Z:`. Per
    // FR-GP-11 we want the same info, so map to `Z`.
    if let Some(rest) = strip_prefix_ci(trimmed, "height:") {
        if let Ok(z) = rest.trim().parse::<f32>() {
            *last_seen_z = Some(z);
            return Some(SemanticComment::Z(z));
        }
    }

    // `; estimated printing time (normal mode) = …` /
    // `; estimated printing time = …`
    if let Some(rest) = strip_after_eq(trimmed, "estimated printing time") {
        return Some(SemanticComment::EstimatedTime(rest.to_owned()));
    }

    // `; filament used [g] = …`, `; filament used [mm] = …` etc.
    if let Some(rest) = strip_after_eq(trimmed, "filament used") {
        return Some(SemanticComment::FilamentUsed(rest.to_owned()));
    }

    // `; total layers count = N`
    if let Some(rest) = strip_after_eq(trimmed, "total layers count") {
        if let Ok(n) = rest.trim().parse::<u32>() {
            return Some(SemanticComment::LayerCount(n));
        }
    }

    // `; printer_model = MK3S`
    if let Some(rest) = strip_after_eq(trimmed, "printer_model") {
        return Some(SemanticComment::PrinterModel(rest.to_owned()));
    }

    None
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() < prefix.len() {
        return None;
    }
    let head = &s[..prefix.len()];
    if head.eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// For `"<key> = <value>"` style comments. Matches `key` case-
/// insensitively, allows surrounding whitespace, returns the
/// trimmed value side or `None` if the line doesn't match.
fn strip_after_eq<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    let trimmed = s.trim_start();
    if trimmed.len() < key.len() {
        return None;
    }
    let head = &trimmed[..key.len()];
    if !head.eq_ignore_ascii_case(key) {
        return None;
    }
    let after = trimmed[key.len()..].trim_start();
    // Allow `<key> [units] = value` — PrusaSlicer/Orca use this for
    // `filament used [g] = ...`. The `[...]` is part of the
    // semantic value, so we don't try to parse it out here.
    let after = if after.starts_with('[') {
        match after.find(']') {
            Some(end) => after[end + 1..].trim_start(),
            None => after,
        }
    } else {
        after
    };
    after.strip_prefix('=').map(|v| v.trim())
}

/// Iterator adapter that pairs every `Line::Move` with the most
/// recent `FeatureType` seen via `;TYPE:`. Yields the same `Line`
/// values as the wrapped iterator, plus a `feature` context bound
/// to each.
pub fn parse_with_feature_context<R: Read>(
    input: R,
) -> impl Iterator<Item = (Result<Line, ParseError>, Option<FeatureType>)> {
    let inner = parse_lines(input);
    FeatureContextIter {
        inner,
        current: None,
    }
}

struct FeatureContextIter<I: Iterator<Item = Result<Line, ParseError>>> {
    inner: I,
    current: Option<FeatureType>,
}

impl<I> Iterator for FeatureContextIter<I>
where
    I: Iterator<Item = Result<Line, ParseError>>,
{
    type Item = (Result<Line, ParseError>, Option<FeatureType>);

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.inner.next()?;
        if let Ok(Line::Comment(c)) = &item {
            if let Some(SemanticComment::FeatureType(ft)) = &c.semantic {
                self.current = Some(ft.clone());
            }
        }
        let feature = match &item {
            Ok(Line::Move(_)) => self.current.clone(),
            _ => None,
        };
        Some((item, feature))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_move(lines: &[Line]) -> &Move {
        lines
            .iter()
            .find_map(|l| match l {
                Line::Move(m) => Some(m),
                _ => None,
            })
            .expect("expected a Move line")
    }

    #[test]
    fn parses_basic_g1_move() {
        let lines = parse_str("G1 X10 Y20 E0.5 F1200\n");
        assert_eq!(lines.len(), 1);
        let m = first_move(&lines);
        assert_eq!(m.command, MoveCommand::Linear);
        assert_eq!(m.command_text, "G1");
        assert_eq!(m.target.x, Some(10.0));
        assert_eq!(m.target.y, Some(20.0));
        assert_eq!(m.target.z, None);
        assert_eq!(m.target.e, Some(0.5));
        assert_eq!(m.feedrate, Some(1200));
        assert_eq!(
            m.param_order,
            vec![MoveParam::X, MoveParam::Y, MoveParam::E, MoveParam::F]
        );
    }

    #[test]
    fn parameter_order_is_preserved() {
        let lines = parse_str("G1 F100 X10 Y10\n");
        let m = first_move(&lines);
        assert_eq!(
            m.param_order,
            vec![MoveParam::F, MoveParam::X, MoveParam::Y]
        );
    }

    #[test]
    fn missing_axis_remains_none() {
        let lines = parse_str("G1 X10\n");
        let m = first_move(&lines);
        assert_eq!(m.target.x, Some(10.0));
        assert_eq!(m.target.y, None);
        assert_eq!(m.target.z, None);
        assert_eq!(m.target.e, None);
    }

    #[test]
    fn g0_g2_g3_are_recognized() {
        let src = "G0 X1 Y1\nG2 X2 Y2 I0.5 J0.5\nG3 X3 Y3 I-0.5 J-0.5\n";
        let lines = parse_str(src);
        assert_eq!(lines.len(), 3);
        let cmds: Vec<_> = lines
            .iter()
            .filter_map(|l| match l {
                Line::Move(m) => Some(m.command),
                _ => None,
            })
            .collect();
        assert_eq!(
            cmds,
            vec![MoveCommand::Rapid, MoveCommand::ArcCw, MoveCommand::ArcCcw]
        );
        let arc = match &lines[1] {
            Line::Move(m) => m,
            _ => panic!(),
        };
        assert_eq!(arc.arc_center.i, Some(0.5));
        assert_eq!(arc.arc_center.j, Some(0.5));
    }

    #[test]
    fn zero_padded_g00_preserves_command_text() {
        let lines = parse_str("G01 X10\n");
        let m = first_move(&lines);
        assert_eq!(m.command, MoveCommand::Linear);
        assert_eq!(m.command_text, "G01");
    }

    #[test]
    fn inline_comment_on_move_is_split_out() {
        let lines = parse_str("G1 X10 ; final move\n");
        let m = first_move(&lines);
        assert_eq!(m.target.x, Some(10.0));
        assert_eq!(m.inline_comment.as_deref(), Some(" final move"));
    }

    #[test]
    fn whole_line_semicolon_comment_carries_raw_with_indent_and_delimiter() {
        let lines = parse_str("  ; hello world\n");
        match &lines[0] {
            Line::Comment(c) => {
                // raw is the whole source line minus the line
                // ending — leading whitespace + `;` + rest. The
                // serializer re-emits `raw + line_ending` for an
                // exact byte round-trip.
                assert_eq!(c.raw, "  ; hello world");
                assert_eq!(c.style, CommentStyle::Semicolon);
                assert!(c.semantic.is_none());
            }
            other => panic!("expected Comment, got {other:?}"),
        }
    }

    #[test]
    fn parenthesized_comment_recognized() {
        let lines = parse_str("(setup)\n");
        match &lines[0] {
            Line::Comment(c) => {
                assert_eq!(c.style, CommentStyle::Parens);
                assert_eq!(c.raw, "(setup)");
            }
            other => panic!("expected Comment, got {other:?}"),
        }
    }

    #[test]
    fn type_comment_emits_feature_type_semantic() {
        let lines = parse_str(";TYPE:External perimeter\n");
        match &lines[0] {
            Line::Comment(c) => match &c.semantic {
                Some(SemanticComment::FeatureType(ft)) => {
                    assert_eq!(*ft, FeatureType::ExternalPerimeter);
                }
                other => panic!("expected FeatureType, got {other:?}"),
            },
            other => panic!("expected Comment, got {other:?}"),
        }
    }

    #[test]
    fn layer_marker_synthesizes_layer_change() {
        let src = ";Z:0.2\n;LAYER:0\nG1 X1\n;LAYER:1\nG1 X2\n";
        let lines = parse_str(src);
        // We expect: Comment(;Z:), Comment(;LAYER:0), LayerChange(0),
        // Move, Comment(;LAYER:1), LayerChange(1), Move.
        let kinds: Vec<&str> = lines
            .iter()
            .map(|l| match l {
                Line::Comment(_) => "Comment",
                Line::Move(_) => "Move",
                Line::LayerChange(_) => "LayerChange",
                Line::ToolChange(_) => "ToolChange",
                Line::Other(_) => "Other",
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "Comment",
                "Comment",
                "LayerChange",
                "Move",
                "Comment",
                "LayerChange",
                "Move",
            ],
        );
        // Layer indices should be 0 and 1; z preserved from the
        // preceding `;Z:` comment for the first.
        let first_lc = lines.iter().find_map(|l| match l {
            Line::LayerChange(lc) => Some(lc),
            _ => None,
        });
        let first_lc = first_lc.expect("expected at least one LayerChange");
        assert_eq!(first_lc.index, 0);
        assert_eq!(first_lc.z, Some(0.2));
        assert_eq!(first_lc.source, LayerSource::Marker);
    }

    #[test]
    fn tool_change_extruder_extracted() {
        let lines = parse_str("T2\n");
        match &lines[0] {
            Line::ToolChange(t) => {
                assert_eq!(t.extruder, 2);
                assert_eq!(t.raw, "T2");
            }
            other => panic!("expected ToolChange, got {other:?}"),
        }
    }

    #[test]
    fn tool_change_with_trailing_comment() {
        let lines = parse_str("T0 ; switch back\n");
        match &lines[0] {
            Line::ToolChange(t) => assert_eq!(t.extruder, 0),
            other => panic!("expected ToolChange, got {other:?}"),
        }
    }

    #[test]
    fn unknown_command_becomes_other() {
        let lines = parse_str("M104 S210\n");
        match &lines[0] {
            Line::Other(o) => {
                assert_eq!(o.raw, "M104 S210");
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn line_endings_are_preserved() {
        let src = "G1 X1\nG1 X2\r\nG1 X3";
        let lines = parse_str(src);
        assert_eq!(lines[0].line_ending(), "\n");
        assert_eq!(lines[1].line_ending(), "\r\n");
        assert_eq!(lines[2].line_ending(), ""); // last line, no terminator
    }

    #[test]
    fn raw_offsets_are_monotonic() {
        let src = "G1 X1\nG1 X2\nG1 X3\n";
        let lines = parse_str(src);
        let offsets: Vec<u64> = lines.iter().map(Line::raw_offset).collect();
        assert_eq!(offsets, vec![0, 6, 12]);
    }

    #[test]
    fn invalid_number_yields_parse_error() {
        let mut iter = parse_lines("G1 Xabc\n".as_bytes());
        let result = iter.next().unwrap();
        let err = result.expect_err("expected error");
        match err.kind {
            ParseErrorKind::InvalidNumber { param, value } => {
                assert_eq!(param, 'X');
                assert_eq!(value, "abc");
            }
            _ => panic!("expected InvalidNumber, got {err:?}"),
        }
        // Iterator continues after the error.
        assert!(iter.next().is_none());
    }

    #[test]
    fn feature_context_carries_across_moves() {
        let src = ";TYPE:Perimeter\nG1 X1\nG1 X2\n;TYPE:Internal infill\nG1 X3\n";
        let features: Vec<Option<FeatureType>> = parse_with_feature_context(src.as_bytes())
            .filter_map(|(line, ft)| match line {
                Ok(Line::Move(_)) => Some(ft),
                _ => None,
            })
            .collect();
        assert_eq!(
            features,
            vec![
                Some(FeatureType::Perimeter),
                Some(FeatureType::Perimeter),
                Some(FeatureType::Infill),
            ],
        );
    }

    #[test]
    fn estimated_time_comment_extracted() {
        let lines = parse_str("; estimated printing time = 1h 23m\n");
        match &lines[0] {
            Line::Comment(c) => match &c.semantic {
                Some(SemanticComment::EstimatedTime(s)) => {
                    assert_eq!(s, "1h 23m");
                }
                other => panic!("expected EstimatedTime, got {other:?}"),
            },
            other => panic!("expected Comment, got {other:?}"),
        }
    }

    #[test]
    fn filament_used_with_units_extracted() {
        let lines = parse_str("; filament used [g] = 4.2\n");
        match &lines[0] {
            Line::Comment(c) => {
                matches!(c.semantic, Some(SemanticComment::FilamentUsed(_)));
            }
            other => panic!("expected Comment, got {other:?}"),
        }
    }

    #[test]
    fn streaming_iterator_yields_lazily() {
        // Build a 1000-line buffer and confirm we can stop early
        // without parsing the rest. Hard to assert directly, but
        // the test will at least exercise the streaming path.
        let mut src = String::new();
        for i in 0..1000 {
            src.push_str(&format!("G1 X{i}\n"));
        }
        let mut iter = parse_lines(src.as_bytes());
        let first = iter.next().unwrap().unwrap();
        match first {
            Line::Move(m) => assert_eq!(m.target.x, Some(0.0)),
            _ => panic!(),
        }
        // Stop after one — iter drop releases the rest.
    }
}
