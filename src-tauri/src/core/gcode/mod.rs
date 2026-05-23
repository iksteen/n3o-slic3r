//! Typed G-code model, parser, and serializer.
//!
//! Shared by the G-code preview (Phase 6) and the plugin system
//! (Phase 8). The model is a typed sequence of `Move` / `Comment` /
//! `LayerChange` / `ToolChange` / `Other` so plugins don't see raw
//! strings and the previewer doesn't reparse on every operation.
//!
//! Owns FR-GP-1 through FR-GP-12 (PRD §6.6) and FR-PL-4 (PRD §6.9).
//! Implementation lands in Phase 3 (parser) and Phase 6 (preview
//! features).

pub mod header;
pub mod model;
pub mod parser;
pub mod serializer;

pub use model::{
    ArcCenter, Comment, CommentStyle, FeatureType, LayerChange, LayerSource, Line, Move,
    MoveCommand, MoveParam, Other, Position, SemanticComment, ToolChange,
};
pub use parser::{parse_lines, parse_str, parse_with_feature_context, ParseError, ParseErrorKind};
pub use header::{
    parse_all_metadata, parse_header, parse_header_str, FilamentUsage,
    HeaderMetadata, SlicerOrigin,
};
pub use serializer::{to_string, write_lines};
