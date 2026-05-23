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

pub mod model;

pub use model::{
    ArcCenter, Comment, CommentStyle, FeatureType, LayerChange, LayerSource, Line, Move,
    MoveCommand, MoveParam, Other, Position, SemanticComment, ToolChange,
};
