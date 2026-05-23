//! G-code preview IR + helpers (Phase 6).
//!
//! Pure-Rust representation of a sliced G-code file in a form the
//! preview renderer (PR-6-8) can upload to GPU buffers without
//! re-parsing. The Phase 3 typed model
//! ([`crate::core::gcode::Line`]) is the input; this module
//! produces a [`PreviewGeometry`] of extrusion + travel segments,
//! plus per-layer ranges + a bounding box.
//!
//! What's here (PR-6-4):
//!
//! - [`ir`] — [`PreviewGeometry`], [`SegmentSet`],
//!   [`LayerRange`], [`RetractionMarker`], [`BoundingBox`].
//! - [`build`] — [`build_preview`] walks `&[Line]` into a
//!   `PreviewGeometry`.
//!
//! What's NOT here yet:
//!
//! - Color encoders (PR-6-5) → consume [`SegmentSet`]'s
//!   `feature` / `speed` / `flow` / `tool` arrays to produce
//!   per-vertex RGB buffers.
//! - Stats computation (PR-6-6) → walks the IR to build
//!   per-layer + full-job summaries.
//! - Tauri commands (PR-6-7) → binary buffer layout for the
//!   IPC wire.

pub mod build;
pub mod ir;

pub use build::build_preview;
pub use ir::{
    BoundingBox, LayerRange, PreviewGeometry, RetractionMarker, SegmentSet,
};
