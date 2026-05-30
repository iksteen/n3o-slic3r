//! Preview IR types — the renderable representation of a sliced
//! G-code file.
//!
//! Designed so the frontend (`GcodePreview`) can pack the
//! payload into Three.js `BufferGeometry` attributes with minimal
//! transformation:
//!
//! - `positions` → `position` attribute (3 floats per vertex,
//!   2 vertices per segment = 6 floats per segment).
//! - `layer_index` → `aLayer` attribute, one float per vertex,
//!   the same value duplicated for both vertices of a segment so
//!   the GPU shader-uniform layer-cull works per-segment.
//! - `feature`, `speed`, `flow`, `tool` → per-segment scalars the
//!   color encoder maps to per-vertex colors.
//! - `source_line` → per-segment back-reference to the original
//!   `gcode::Line` index, used by the hover-inspection raycast
//!   to look up the source command.

use serde::{Deserialize, Serialize};

use crate::core::gcode::FeatureType;

/// Self-contained renderable representation of a sliced G-code
/// file. Built once per load by [`super::build_preview`]; the
/// renderer caches it and swaps color attributes separately via
/// encoders.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PreviewGeometry {
    pub extrusions: SegmentSet,
    pub travels: SegmentSet,
    pub retractions: Vec<RetractionMarker>,
    /// Layer index → first/last segment-index range in
    /// `extrusions`. End is exclusive. Used by stats and
    /// the CPU-side fallback of the layer slider.
    pub layer_ranges: Vec<LayerRange>,
    /// Computed over extrusion segment endpoints only — travels
    /// (especially the wide skirt loops) would otherwise pull the
    /// bbox unhelpfully large.
    pub bounding_box: BoundingBox,
}

/// Parallel arrays describing every segment of one kind
/// (extrusions OR travels). Indexing by segment id `i`:
///
/// - `positions[6*i .. 6*i+6]` = `[start.x, start.y, start.z,
///   end.x, end.y, end.z]`.
/// - `layer_index[2*i .. 2*i+2]` = duplicated per-vertex.
/// - `feature[i]`, `speed[i]`, `flow[i]`, `tool[i]`,
///   `source_line[i]` — per-segment scalars.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SegmentSet {
    /// Per-vertex 3D positions. 6 floats per segment (2 vertices
    /// × 3 components). World-space, millimeters, printer-bed
    /// coordinates.
    pub positions: Vec<f32>,
    /// Per-vertex layer index. Same value duplicated for both
    /// vertices of a segment; f32 so the GPU can bind it as a
    /// vertex attribute with the rest.
    pub layer_index: Vec<f32>,
    /// Feature type for each segment (extrusion only — travels
    /// are always `FeatureType::Travel`). Length == segment count.
    pub feature: Vec<FeatureType>,
    /// Feed-rate at the time the segment was emitted, mm/s
    /// (converted from libslic3r's mm/min).
    pub speed: Vec<f32>,
    /// Volumetric flow, mm³/s. Computed from `ΔE × filament
    /// cross-section / segment duration`. For travels this is 0.
    pub flow: Vec<f32>,
    /// 0-based tool index (`T0` → `0`). Tracks the most recent
    /// `ToolChange` line. 0 when no tool change has happened.
    pub tool: Vec<u8>,
    /// Back-reference to the original `gcode::Line` index. Used
    /// by hover inspection to surface the source
    /// command for a raycast-hit segment.
    pub source_line: Vec<u32>,
}

impl SegmentSet {
    /// Number of segments in this set. Derived from
    /// `positions.len() / 6`; the per-segment arrays must match.
    pub fn len(&self) -> usize {
        self.positions.len() / 6
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Push one segment with all its per-vertex + per-segment
    /// fields in lockstep. Keeps the parallel-arrays invariant
    /// out of build_preview's main loop.
    pub(crate) fn push(&mut self, seg: Segment) {
        self.positions.extend_from_slice(&[
            seg.start[0],
            seg.start[1],
            seg.start[2],
            seg.end[0],
            seg.end[1],
            seg.end[2],
        ]);
        self.layer_index.push(seg.layer as f32);
        self.layer_index.push(seg.layer as f32);
        self.feature.push(seg.feature);
        self.speed.push(seg.speed);
        self.flow.push(seg.flow);
        self.tool.push(seg.tool);
        self.source_line.push(seg.source_line);
    }
}

/// Internal builder helper — one segment's worth of data the
/// dispatch loop hands to [`SegmentSet::push`]. Not part of the
/// public IR (the rendered shape is the parallel arrays).
pub(crate) struct Segment {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub layer: u32,
    pub feature: FeatureType,
    pub speed: f32,
    pub flow: f32,
    pub tool: u8,
    pub source_line: u32,
}

/// Marker for a retract point — `E` decreased without an XY move.
/// Used by retraction-visibility toggle: rendered as a
/// small red dot at `position` when shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetractionMarker {
    pub position: [f32; 3],
    pub layer_index: u32,
    /// Magnitude of the retract in mm of filament (positive).
    pub amount_mm: f32,
}

/// One layer's slice of the extrusions array. `segment_start ..
/// segment_end` (end exclusive) is the half-open range of
/// extrusion indices that belong to this layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerRange {
    pub layer_index: u32,
    /// Z height at the top of this layer (mm). When the parser
    /// detected the layer change without a `;Z:` marker (heuristic
    /// path), this is the printer's current Z when the change
    /// fired.
    pub z: f32,
    pub segment_start: u32,
    pub segment_end: u32,
}

/// Axis-aligned bounding box. `min` and `max` are `[x, y, z]` in
/// the same units as `positions` (mm in printer space).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Default for BoundingBox {
    /// Initially-degenerate bbox that grows via [`Self::extend`]
    /// on each extrusion endpoint. `min` starts at +∞, `max` at
    /// −∞ so the first point sets both to that point.
    fn default() -> Self {
        Self {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        }
    }
}

impl BoundingBox {
    pub fn extend(&mut self, p: [f32; 3]) {
        for axis in 0..3 {
            if p[axis] < self.min[axis] {
                self.min[axis] = p[axis];
            }
            if p[axis] > self.max[axis] {
                self.max[axis] = p[axis];
            }
        }
    }

    /// `true` when no point has been [`extend`]ed into the bbox.
    /// Lets the caller distinguish "empty preview" from "preview
    /// at the origin".
    pub fn is_empty(&self) -> bool {
        self.min[0] > self.max[0]
    }
}
