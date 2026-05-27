//! G-code preview IR + helpers (Phase 6).
//!
//! Pure-Rust representation of a sliced G-code file in a form the
//! preview renderer can upload to GPU buffers without
//! re-parsing. The Phase 3 typed model
//! ([`crate::core::gcode::Line`]) is the input; this module
//! produces a [`PreviewGeometry`] of extrusion + travel segments,
//! plus per-layer ranges + a bounding box.
//!
//! What's here:
//!
//! - [`ir`] — [`PreviewGeometry`], [`SegmentSet`],
//!   [`LayerRange`], [`RetractionMarker`], [`BoundingBox`].
//! - [`build`] — [`build_preview`] walks `&[Line]` into a
//!   `PreviewGeometry`.
//!
//! What's NOT here yet:
//!
//! - Color encoders → consume [`SegmentSet`]'s
//!   `feature` / `speed` / `flow` / `tool` arrays to produce
//!   per-vertex RGB buffers.
//! - Stats computation → walks the IR to build
//!   per-layer + full-job summaries.
//! - Tauri commands → binary buffer layout for the
//!   IPC wire.

pub mod build;
pub mod colors;
pub mod commands;
pub mod ir;
pub mod registry;
pub mod stats;

pub use build::build_preview;
pub use colors::{encode_colors, ColorMode, Palette};
pub use commands::{
    preview_buffers, preview_drop, preview_layer_stats, preview_load,
    preview_segment_detail, PreviewLoadResponse, SegmentDetail,
};
pub use ir::{
    BoundingBox, LayerRange, PreviewGeometry, RetractionMarker, SegmentSet,
};
pub use registry::{LoadedPreview, PreviewHandle, PreviewRegistry};
pub use stats::{
    compute_job_stats, compute_layer_stats, layer_time_map, FullJobStats,
    HeightStats, PerLayerStats,
};
