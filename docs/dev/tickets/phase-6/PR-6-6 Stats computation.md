# PR-6-6 — Per-layer + full-job stats computation

Status: ✅ shipped.

**Scope.** Aggregate stats from the PR-6-4 `PreviewGeometry`
into per-layer and full-job summaries the stats panels (PR-6-12)
render. Also produces the per-layer-time map the `LayerTime`
color mode (PR-6-5) consumes.

**Acceptance criteria.**

- New module `core/preview/stats.rs`. Suggested surface:
  ```rust
  pub struct PerLayerStats {
      pub layer_index: u32,
      pub z: f32,
      pub layer_height: f32,
      pub duration_seconds: f32,
      pub max_speed: f32,
      pub filament_used_mm: HashMap<u8, f32>,  // by tool index
      pub feature_breakdown: HashMap<FeatureType, f32>,  // seconds per feature
  }

  pub struct FullJobStats {
      pub total_duration_seconds: f32,
      pub layer_count: u32,
      pub feature_breakdown: HashMap<FeatureType, f32>,  // total seconds
      pub filament_used_mm: HashMap<u8, f32>,
      pub bounding_box: BoundingBox,
      pub layer_heights: HeightStats,
  }

  pub struct HeightStats {
      pub min: f32,
      pub max: f32,
      pub variable: bool,  // min != max within tolerance
  }

  pub fn compute_layer_stats(
      geometry: &PreviewGeometry,
  ) -> Vec<PerLayerStats>;

  pub fn compute_job_stats(
      geometry: &PreviewGeometry,
      layer_stats: &[PerLayerStats],
  ) -> FullJobStats;

  /// Convenience: pull per-layer time into a flat array
  /// `layer_times[layer_index] = duration_seconds`. Consumed
  /// by PR-6-5's `LayerTime` color mode.
  pub fn layer_time_map(layer_stats: &[PerLayerStats]) -> Vec<f32>;
  ```

- **Time integration:** for each extrusion + travel segment,
  `duration = length_mm / speed_mm_s`. Sum per layer for
  layer time; sum across layers for total job time.
  - Travel segments count toward job time (they're real
    motion); the renderer shows them dimmed.
  - Retractions add a fixed ~retract-time estimate (use
    `retract_speed` from the gcode header if available;
    fallback to a 0.5s assumption per retract).

- **Layer height** comes from the Z difference between
  consecutive `LayerRange.z` entries. The first layer's
  height comes from the gcode header's `first_layer_height`
  if present, else the configured `layer_height`, else 0.2mm
  fallback.

- **Variable-layer-height detection:** `HeightStats.variable`
  is `true` when `max - min > 0.005mm`. This surfaces variable-
  layer-height surprises the user might want to spot before
  printing third-party G-code.

- **Filament used** per tool: integrate `delta_E_mm` per
  segment grouped by `segments.tool[i]`. Travels with
  retractions have negative E delta; ignore (don't subtract
  from totals — extrusion is what was actually laid down).

- **Bounding box:** copy from `geometry.bounding_box` (already
  computed in PR-6-4).

- Tests:
  - **Synthetic 2-layer fixture:** authored G-code with two
    distinct layer heights → `compute_layer_stats` returns
    `[{layer_height: 0.2}, {layer_height: 0.16}]`.
  - **Feature breakdown sums to layer duration:** for each
    `PerLayerStats`, `sum(feature_breakdown.values()) ≈
    duration_seconds`.
  - **Job duration = sum of layer durations** within 1%
    tolerance.
  - **Multi-tool filament tracking:** a 2-tool fixture with
    half the extrusion on each tool produces a
    `filament_used_mm` map with both tool keys.
  - **Variable-height detection** fires correctly on
    authored variable-height fixture.

- **Perf:** 3M segments → all stats in < 200ms on dev
  hardware. Criterion bench in PR-6-16.

**Effort.** ~1.5 days. Each per-layer summary takes one walk
over the segment array; job stats is a fold over per-layer.

**Dependencies.** PR-6-4 (`PreviewGeometry` + `SegmentSet`
shape + `LayerRange`).

**Out of scope.**

- Filament weight (grams) — requires filament density from
  the cascade. Surface as a Phase 9 polish; for now,
  filament_used_mm + per-extruder material identity is
  enough for the panel.
- Time-per-feature-per-layer (per-layer features pie chart) —
  Phase 9 polish. The aggregate per-layer-time is sufficient.
- Estimated-vs-actual time comparison — Phase 7 driver work
  feeds the "actual" half once real prints land.

**Cut candidate.** Per-layer stats → save ~1 day. Keep
full-job stats only. Per Exec Plan cut list. **Not
recommended** — user signed off on per-layer panels.
