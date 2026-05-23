# PR-6-4 — Preview IR (segment build from `gcode::Line`)

Status: ❌ open.

**Scope.** Define the in-memory representation the preview
renderer consumes, and the pure function that builds it from
the Phase 3 typed G-code model. Foundation for the color
encoders (PR-6-5), stats (PR-6-6), and Tauri buffers (PR-6-7).

**Acceptance criteria.**

- New module `core/preview/`. Suggested layout:
  ```
  core/preview/
    mod.rs            # re-exports + module docs
    ir.rs             # PreviewGeometry types
    build.rs          # build_preview(&[Line]) → PreviewGeometry
  ```

- Core types (Serializable for the Tauri buffer wire format
  in PR-6-7):
  ```rust
  /// Self-contained renderable representation of a sliced
  /// G-code file. Built once per load; color attributes
  /// swap separately (see PR-6-5).
  pub struct PreviewGeometry {
      pub extrusions: SegmentSet,
      pub travels: SegmentSet,
      pub retractions: Vec<RetractionMarker>,
      /// Layer index → first/last segment index range in
      /// `extrusions`. Used by stats (PR-6-6) + slider's
      /// up-to-N draw-range path (PR-6-9 fallback).
      pub layer_ranges: Vec<LayerRange>,
      pub bounding_box: BoundingBox,
  }

  pub struct SegmentSet {
      /// Per-vertex 3D positions (x, y, z floats interleaved,
      /// 2 vertices per segment).
      pub positions: Vec<f32>,
      /// Per-vertex layer index (one f32 per vertex,
      /// repeated for both vertices of a segment).
      pub layer_index: Vec<f32>,
      /// Per-segment feature type. Length = positions.len() / 6.
      pub feature: Vec<FeatureType>,
      /// Per-segment feed rate (mm/s). Same length.
      pub speed: Vec<f32>,
      /// Per-segment flow rate (mm³/s). Same length.
      pub flow: Vec<f32>,
      /// Per-segment tool index (0-based). Same length.
      pub tool: Vec<u8>,
      /// Per-segment back-reference to the gcode::Line index
      /// in the source `Vec<Line>` (used for hover inspection
      /// — PR-6-11).
      pub source_line: Vec<u32>,
  }

  pub struct RetractionMarker {
      pub position: [f32; 3],
      pub layer_index: u32,
      pub amount_mm: f32,
  }

  pub struct LayerRange {
      pub layer_index: u32,
      pub z: f32,
      pub segment_start: u32,
      pub segment_end: u32,  // exclusive
  }

  pub struct BoundingBox {
      pub min: [f32; 3],
      pub max: [f32; 3],
  }
  ```

- `pub fn build_preview(lines: &[gcode::Line]) -> PreviewGeometry`:
  - Walks the line stream. Tracks current position
    (X/Y/Z/E), current feed rate, current tool, current
    layer index, current feature type (from
    `SemanticComment::FeatureType`).
  - Each `Line::Move` produces one segment from previous
    position to new position. Classifies as extrusion (E
    increasing) or travel (E unchanged or decreasing).
    Retractions (E decreasing without XY motion) populate
    `retractions`, not segments.
  - Layer transitions (from `Line::LayerChange`) bump the
    layer index + push a new `LayerRange`.
  - Tool changes (from `Line::ToolChange`) update current
    tool.
  - Feature transitions update current feature.
  - Bounding box updates per extrusion segment endpoint
    (travels are excluded from bbox to avoid skirts pulling
    bbox huge).

- **Determinism + ordering:** segment order matches G-code
  command order. `source_line` is monotonically increasing
  within `extrusions`. The hover-inspection raycast (PR-6-11)
  depends on this.

- Tests:
  - **Synthetic fixture:** hand-author a short G-code string
    (header + a few moves + a layer change), parse via
    `gcode::parse_str`, build preview, assert exact segment
    count, exact layer ranges, exact bounding box.
  - **Real fixture:** load `phase-3-smoke`'s output G-code,
    assert layer count matches the slicer's reported layer
    count from the gcode header.
  - **Travel vs extrusion classification:** a move with `E`
    increase becomes an extrusion; same XY without E becomes
    a travel; E decrease without XY becomes a retraction
    (not a segment).
  - **Tool tracking:** a `T0 … T1 … T0` sequence produces
    segments with the correct per-segment tool index.

- **Perf:** parse + build a 5MB G-code in < 500ms on the dev
  hardware. Pre-allocate `Vec`s with reasonable capacities
  (a worst-case 50MB G-code has ~5M lines → ~3M extrusions →
  ~36M `positions` floats). Criterion benchmark sits in
  PR-6-16, but a basic `#[ignore]` test for the dev machine
  is fine here.

**Effort.** ~1.5 days. Most of the work is the state machine
walking the line stream; the IR shape is simple.

**Dependencies.** Phase 3 `gcode::{Line, model::*}` (already
shipped).

**Out of scope.**

- Color computation (PR-6-5 consumes `feature`, `speed`,
  `flow`, `tool` arrays).
- Stats aggregation (PR-6-6).
- Tauri wire format (PR-6-7 packs `SegmentSet` into binary
  buffers).
- Renderer (PR-6-8).
- Source-line cross-reference resolution for hover (PR-6-11
  walks the original `Vec<Line>` using `source_line`
  back-refs).

**Cut candidate.** None — every later preview ticket depends
on this IR.
