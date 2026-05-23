//! Typed G-code line model (PR-3-5).
//!
//! The contract between PR-3-6's parser and PR-3-7's serializer —
//! and the surface every downstream consumer (Phase 6 preview,
//! Phase 8 plugins) reads. Per FR-PL-4, plugins see a typed
//! sequence of `Move` / `Comment` / `LayerChange` / `ToolChange` /
//! `Other`, never raw strings.
//!
//! The model is byte-precise enough to let PR-3-7's serializer
//! round-trip the parser's output byte-for-byte. That's the
//! project's independent oracle (Execution Plan §5 exit criteria);
//! every field exists so a re-serialized line is indistinguishable
//! from its source. Specifically:
//!
//! - `Move.param_order` captures which letters appeared in which
//!   order so `G1 F100 X10 Y10` and `G1 X10 Y10 F100` (same
//!   semantics, different ordering) round-trip distinctly.
//! - `Move.command_text` preserves the exact command spelling
//!   (`G0` vs `G00`, `G1` vs `G01`) instead of normalizing to one
//!   form.
//! - `Comment.raw` is the canonical re-emit source; the
//!   `semantic` tag is *only* an inspector.
//! - `Other.raw` re-emits verbatim.
//! - Every variant carries its line ending (`"\n"` / `"\r\n"`) so
//!   mixed-ending files round-trip without normalization.
//!
//! Resist adding "string-only" fast paths around this model. The
//! oracle stops working the moment the renderer or a plugin can
//! reach around the typed shape; see `docs/tickets/phase-3.md`
//! for the architecture note.

use serde::{Deserialize, Serialize};

/// One parsed line of G-code.
///
/// Every variant carries:
///
/// - `raw_offset` — the byte offset of the line in the source. Used
///   by error messages and by Phase 6's hover-inspection to point
///   back at the original.
/// - `line_ending` — `"\n"` or `"\r\n"`. Preserved so mixed-ending
///   files re-emit identically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum Line {
    Move(Move),
    Comment(Comment),
    LayerChange(LayerChange),
    ToolChange(ToolChange),
    Other(Other),
}

impl Line {
    pub fn raw_offset(&self) -> u64 {
        match self {
            Self::Move(m) => m.raw_offset,
            Self::Comment(c) => c.raw_offset,
            Self::LayerChange(l) => l.raw_offset,
            Self::ToolChange(t) => t.raw_offset,
            Self::Other(o) => o.raw_offset,
        }
    }

    pub fn line_ending(&self) -> &str {
        match self {
            Self::Move(m) => &m.line_ending,
            Self::Comment(c) => &c.line_ending,
            Self::LayerChange(l) => &l.line_ending,
            Self::ToolChange(t) => &t.line_ending,
            Self::Other(o) => &o.line_ending,
        }
    }
}

/// A `G0` / `G1` / `G2` / `G3` line — the actual extruder motion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Move {
    /// Whole source line excluding the line ending. The serializer
    /// emits `raw + line_ending` for byte-equivalent round-trip;
    /// the typed fields below are inspection / mutation surfaces.
    /// When a plugin mutates a typed field (Phase 8), it's
    /// responsible for invalidating or rewriting `raw` — the
    /// default un-mutated parse→serialize path emits `raw` verbatim.
    pub raw: String,
    /// `G0` / `G1` / `G2` / `G3`. Kept as a typed enum so callers
    /// can switch without string parsing.
    pub command: MoveCommand,
    /// Exact source spelling of the command token (`"G0"`, `"G00"`,
    /// `"G1 "` — including any trailing-space oddities). Preserved
    /// because some firmware-flavor G-code uses zero-padded numbers
    /// and we don't want to normalize them away.
    pub command_text: String,
    /// X/Y/Z/E target. `None` per axis means the parameter was
    /// absent on the source line — that's distinct from `Some(0.0)`
    /// because firmware re-applies the previous value when the
    /// letter is missing.
    pub target: Position,
    /// `F` parameter (feedrate, mm/min). Same `None` semantics.
    pub feedrate: Option<u32>,
    /// `I` / `J` parameters for arc moves (`G2` / `G3`).
    pub arc_center: ArcCenter,
    /// Order the parameter letters appeared in the source line.
    /// `["F", "X", "Y"]` for `G1 F100 X10 Y10`, vs `["X", "Y", "F"]`
    /// for `G1 X10 Y10 F100`. Drives serializer output order so the
    /// round-trip is byte-equal.
    pub param_order: Vec<MoveParam>,
    /// Inline comment after a `;`, if any. `G1 X10 ; final move` →
    /// `inline_comment = Some(" final move")` (the leading space
    /// is preserved). `None` if the line has no `;`.
    pub inline_comment: Option<String>,
    pub raw_offset: u64,
    pub line_ending: String,
}

/// Which G-code motion command this is. Stays in sync with
/// `Move.command_text`'s canonical letter; the *exact* text
/// (zero-padding, casing) lives on `command_text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveCommand {
    /// `G0` — rapid travel (no extrusion in canonical use).
    Rapid,
    /// `G1` — linear interpolated move.
    Linear,
    /// `G2` — clockwise arc.
    ArcCw,
    /// `G3` — counter-clockwise arc.
    ArcCcw,
}

/// X/Y/Z/E target on a Move. Each axis `None` means the parameter
/// was absent (firmware retains the previous value). Wrapping
/// `Option<f32>` per axis is load-bearing for round-trip:
/// `G1 X10 Y20` and `G1 X10 Y20 Z0` differ in firmware behavior
/// and must be preserved.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Position {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub z: Option<f32>,
    pub e: Option<f32>,
}

/// `I` / `J` arc-center offsets for `G2` / `G3`. Both `None` for
/// linear moves.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ArcCenter {
    pub i: Option<f32>,
    pub j: Option<f32>,
}

/// Identifies one parameter letter for serializer ordering. Kept
/// as a typed enum (not a `char`) so the parser/serializer can
/// `match` exhaustively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveParam {
    X,
    Y,
    Z,
    E,
    F,
    I,
    J,
}

/// A `;…` or `(…)` comment line. The `raw` field is the canonical
/// re-emit source — the whole source line minus the line ending,
/// including the leading whitespace and the delimiter. The
/// `semantic` tag is an inspector hint, not a serialization source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    /// Whole source line excluding the line ending. Includes the
    /// leading whitespace AND the delimiter so the serializer can
    /// re-emit identically. For `"  ; foo bar"` → `raw =
    /// "  ; foo bar"`. `style` reports which delimiter convention
    /// the source used; it's redundant for serialization but useful
    /// for plugins that want to filter on delimiter style.
    pub raw: String,
    pub style: CommentStyle,
    /// What kind of structured information, if any, this comment
    /// carries. Recognized at parse time; serializer ignores it
    /// (re-emit uses `raw`). `None` for free-form comments.
    pub semantic: Option<SemanticComment>,
    pub raw_offset: u64,
    pub line_ending: String,
}

/// Comment delimiter style. Marlin/Klipper use both `;` (rest of
/// line) and `(…)` (parenthesized, terminated). RepRap-flavor uses
/// `;` exclusively; we preserve whichever the source used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommentStyle {
    /// `; rest of line`
    Semicolon,
    /// `(parenthesized)`
    Parens,
}

/// Recognized structured comments. Each variant maps a pattern the
/// parser knows about; unknown comments stay as `Comment { semantic:
/// None }`. Extending this enum doesn't break the model — old
/// callers see `None` for new variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum SemanticComment {
    /// `; TYPE: <feature>` (Orca / Bambu / PrusaSlicer). Feeds the
    /// per-move feature context the parser threads through via
    /// `parse_with_feature_context`.
    FeatureType(FeatureType),
    /// `;LAYER:<n>` or `; LAYER_CHANGE` or `;LAYER_CHANGE`.
    Layer(u32),
    /// `;Z:<height>` — current layer Z.
    Z(f32),
    /// `; estimated printing time = <duration>`.
    EstimatedTime(String),
    /// `; filament used [g] = <values>` etc. Raw string preserved;
    /// PR-3-3 + PR-3-8 own the parsing into typed numbers.
    FilamentUsed(String),
    /// `; total layers count = <n>`.
    LayerCount(u32),
    /// `; printer_model = <name>`.
    PrinterModel(String),
    /// `M104 S210` / `M109 S210` extruder temp via comment header
    /// preview (the actual M-command lives in `Other` for now —
    /// Phase 6 may upgrade this).
    ExtruderTemp(f32),
    /// `M140 S60` / `M190 S60` bed temp likewise.
    BedTemp(f32),
}

/// Feature-type classification for a Move. Matches FR-GP-3's list,
/// with an `Other(String)` escape for forward compat. Parses from
/// the Orca / Bambu canonical strings — see `FeatureType::parse`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FeatureType {
    Perimeter,
    ExternalPerimeter,
    Infill,
    SolidInfill,
    TopSolidInfill,
    Bridge,
    Support,
    Skirt,
    Brim,
    Travel,
    /// Anything else the slicer emits; preserves the raw string so
    /// future slicer additions don't lose information.
    Other(String),
}

impl FeatureType {
    /// Parse Orca/Bambu/Prusa-style `TYPE:` tokens. The mapping
    /// table is data-driven rather than hard-coded so adding a new
    /// canonical name later is one line. Unknown values fall
    /// through to `Other(token.to_owned())` rather than erroring;
    /// the parser is lenient about feature labels by design.
    pub fn parse(token: &str) -> Self {
        let normalized = token.trim().to_ascii_lowercase();
        for (canonical, variant) in CANONICAL_FEATURE_TOKENS {
            if normalized == *canonical {
                return variant.clone();
            }
        }
        Self::Other(token.trim().to_owned())
    }

    /// Inverse of `parse` — the canonical token the serializer uses
    /// when emitting a `;TYPE:` comment. Round-trip: a
    /// FeatureType parsed from a recognized token re-emits as that
    /// same canonical form. `Other("...")` round-trips its raw
    /// string verbatim.
    pub fn as_token(&self) -> String {
        match self {
            Self::Perimeter => "Perimeter".into(),
            Self::ExternalPerimeter => "External perimeter".into(),
            Self::Infill => "Internal infill".into(),
            Self::SolidInfill => "Solid infill".into(),
            Self::TopSolidInfill => "Top solid infill".into(),
            Self::Bridge => "Bridge infill".into(),
            Self::Support => "Support material".into(),
            Self::Skirt => "Skirt/Brim".into(),
            Self::Brim => "Skirt/Brim".into(),
            Self::Travel => "Travel".into(),
            Self::Other(s) => s.clone(),
        }
    }
}

/// Canonical lowercase strings → `FeatureType`. Sourced from
/// OrcaSlicer's `GCodeProcessor.cpp` extrusion-role names (the
/// canonical emitter on `;TYPE:` comments). PrusaSlicer's output
/// differs slightly ("Internal infill" vs. "internal-infill"); the
/// lower-cased match covers both.
const CANONICAL_FEATURE_TOKENS: &[(&str, FeatureType)] = &[
    ("perimeter", FeatureType::Perimeter),
    ("external perimeter", FeatureType::ExternalPerimeter),
    ("internal infill", FeatureType::Infill),
    ("infill", FeatureType::Infill),
    ("solid infill", FeatureType::SolidInfill),
    ("top solid infill", FeatureType::TopSolidInfill),
    ("bridge infill", FeatureType::Bridge),
    ("bridge", FeatureType::Bridge),
    ("support material", FeatureType::Support),
    ("support", FeatureType::Support),
    ("skirt/brim", FeatureType::Skirt),
    ("skirt", FeatureType::Skirt),
    ("brim", FeatureType::Brim),
    ("travel", FeatureType::Travel),
];

/// Synthetic line emitted by the parser at layer boundaries. Not a
/// real G-code command — the parser inserts it adjacent to the
/// move that crosses the boundary, and the serializer emits it as
/// the canonical `;LAYER:<n>` comment so the round-trip preserves
/// the boundary marker.
///
/// Detection rules (parser-side, documented here so consumers know
/// what they're getting):
///   1. `; CHANGE_LAYER` / `;LAYER_CHANGE` / `;LAYER:<n>` comment.
///   2. Z-axis advance on a `G0` / `G1` with no extrusion (lift).
///   3. Heuristic retract → travel → lift sequence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerChange {
    /// 0-based layer index. The first printed layer is `0`; the
    /// parser starts counting from the first detected boundary.
    pub index: u32,
    /// World-space Z height the boundary marker reported. `None`
    /// when the boundary was detected via the retract/travel/lift
    /// heuristic and no `;Z:` value was present.
    pub z: Option<f32>,
    /// Whether the boundary came from an explicit comment marker
    /// or the heuristic. The serializer always emits the canonical
    /// `;LAYER:<n>` form regardless.
    pub source: LayerSource,
    pub raw_offset: u64,
    pub line_ending: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerSource {
    /// A `;LAYER:` / `;CHANGE_LAYER` comment in the source.
    Marker,
    /// Detected from G0/G1 Z motion patterns.
    Heuristic,
}

/// `T<n>` tool change command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolChange {
    /// 0-based extruder index. `T0` → `0`.
    pub extruder: u8,
    /// Whole source line excluding the line ending. Includes any
    /// leading whitespace and any trailing `;` inline comment, so
    /// the serializer emits `raw + line_ending` for byte round-trip.
    pub raw: String,
    pub raw_offset: u64,
    pub line_ending: String,
}

/// Catch-all variant — every line the parser didn't classify as a
/// `Move` / `Comment` / `LayerChange` / `ToolChange`. The raw bytes
/// are preserved so the serializer can re-emit untouched.
///
/// Today this absorbs M-commands (`M104`, `M140`, …), unrecognized
/// G-codes, blank lines, firmware-specific extensions, anything
/// the lenient parser doesn't have a typed slot for. Phase 6 may
/// promote some of these (notably temperature M-commands) once the
/// preview needs to ask "what temperature is the bed at this
/// point?" without re-parsing strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Other {
    /// The whole source line *excluding* its line ending.
    pub raw: String,
    pub raw_offset: u64,
    pub line_ending: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_round_trips_via_serde() {
        let m = Move {
            raw: "G1 F1200 X10 Y20 E0.5 ; final".into(),
            command: MoveCommand::Linear,
            command_text: "G1".into(),
            target: Position {
                x: Some(10.0),
                y: Some(20.0),
                z: None,
                e: Some(0.5),
            },
            feedrate: Some(1200),
            arc_center: ArcCenter::default(),
            param_order: vec![MoveParam::F, MoveParam::X, MoveParam::Y, MoveParam::E],
            inline_comment: Some(" final".into()),
            raw_offset: 42,
            line_ending: "\n".into(),
        };
        let line = Line::Move(m.clone());
        let json = serde_json::to_string(&line).unwrap();
        let parsed: Line = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, line);
    }

    #[test]
    fn comment_with_semantic_tag_serializes() {
        let c = Comment {
            raw: "TYPE: Perimeter".into(),
            style: CommentStyle::Semicolon,
            semantic: Some(SemanticComment::FeatureType(FeatureType::Perimeter)),
            raw_offset: 7,
            line_ending: "\n".into(),
        };
        let line = Line::Comment(c.clone());
        let json = serde_json::to_string(&line).unwrap();
        let parsed: Line = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, line);
    }

    #[test]
    fn layer_change_carries_index_and_source() {
        let l = LayerChange {
            index: 12,
            z: Some(2.4),
            source: LayerSource::Marker,
            raw_offset: 1000,
            line_ending: "\n".into(),
        };
        let line = Line::LayerChange(l.clone());
        let json = serde_json::to_string(&line).unwrap();
        let parsed: Line = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, line);
    }

    #[test]
    fn tool_change_preserves_spelling() {
        let t = ToolChange {
            extruder: 2,
            raw: "T2".into(),
            raw_offset: 50,
            line_ending: "\n".into(),
        };
        let line = Line::ToolChange(t.clone());
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(serde_json::from_str::<Line>(&json).unwrap(), line);
    }

    #[test]
    fn other_preserves_raw() {
        let o = Other {
            raw: "M104 S210".into(),
            raw_offset: 0,
            line_ending: "\r\n".into(),
        };
        let line = Line::Other(o.clone());
        assert_eq!(line.line_ending(), "\r\n");
        assert_eq!(line.raw_offset(), 0);
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(serde_json::from_str::<Line>(&json).unwrap(), line);
    }

    #[test]
    fn feature_type_parses_canonical_orca_tokens() {
        assert_eq!(FeatureType::parse("Perimeter"), FeatureType::Perimeter);
        assert_eq!(
            FeatureType::parse("External perimeter"),
            FeatureType::ExternalPerimeter
        );
        assert_eq!(FeatureType::parse("Internal infill"), FeatureType::Infill);
        assert_eq!(FeatureType::parse("infill"), FeatureType::Infill);
        assert_eq!(
            FeatureType::parse("Solid infill"),
            FeatureType::SolidInfill
        );
        assert_eq!(
            FeatureType::parse("Top solid infill"),
            FeatureType::TopSolidInfill
        );
        assert_eq!(
            FeatureType::parse("Bridge infill"),
            FeatureType::Bridge
        );
        assert_eq!(
            FeatureType::parse("Support material"),
            FeatureType::Support
        );
        assert_eq!(FeatureType::parse("Skirt"), FeatureType::Skirt);
        assert_eq!(FeatureType::parse("Brim"), FeatureType::Brim);
        assert_eq!(FeatureType::parse("Travel"), FeatureType::Travel);
    }

    #[test]
    fn feature_type_parse_is_case_and_whitespace_insensitive() {
        assert_eq!(
            FeatureType::parse("  PERIMETER  "),
            FeatureType::Perimeter
        );
        assert_eq!(
            FeatureType::parse("external perimeter"),
            FeatureType::ExternalPerimeter
        );
    }

    #[test]
    fn feature_type_other_preserves_unknown_label() {
        let unknown = FeatureType::parse("Custom thing");
        assert_eq!(unknown, FeatureType::Other("Custom thing".into()));
        assert_eq!(unknown.as_token(), "Custom thing");
    }

    #[test]
    fn raw_offset_and_line_ending_accessors_work_for_each_variant() {
        let m = Line::Move(Move {
            raw: "G0".into(),
            command: MoveCommand::Rapid,
            command_text: "G0".into(),
            target: Position::default(),
            feedrate: None,
            arc_center: ArcCenter::default(),
            param_order: vec![],
            inline_comment: None,
            raw_offset: 1,
            line_ending: "\n".into(),
        });
        let c = Line::Comment(Comment {
            raw: "".into(),
            style: CommentStyle::Semicolon,
            semantic: None,
            raw_offset: 2,
            line_ending: "\r\n".into(),
        });
        let l = Line::LayerChange(LayerChange {
            index: 0,
            z: None,
            source: LayerSource::Heuristic,
            raw_offset: 3,
            line_ending: "\n".into(),
        });
        let t = Line::ToolChange(ToolChange {
            extruder: 0,
            raw: "T0".into(),
            raw_offset: 4,
            line_ending: "\n".into(),
        });
        let o = Line::Other(Other {
            raw: "M0".into(),
            raw_offset: 5,
            line_ending: "\n".into(),
        });

        for (line, off, ending) in [
            (m, 1, "\n"),
            (c, 2, "\r\n"),
            (l, 3, "\n"),
            (t, 4, "\n"),
            (o, 5, "\n"),
        ] {
            assert_eq!(line.raw_offset(), off);
            assert_eq!(line.line_ending(), ending);
        }
    }
}
