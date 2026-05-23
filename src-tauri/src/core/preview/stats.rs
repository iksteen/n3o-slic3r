//! Preview stats aggregation (PR-6-6).
//!
//! Walks the [`PreviewGeometry`] from PR-6-4 into per-layer and
//! full-job summaries the stats panels (PR-6-12) render, plus a
//! flat layer-time map the `ColorMode::LayerTime` encoder (PR-6-5)
//! consumes.
//!
//! Pure functions over the IR — no I/O, no parsing. Costs:
//! O(segments + layers) for `compute_layer_stats`; O(layers) for
//! `compute_job_stats`. A 50MB G-code → ~3M segments → ~200ms on
//! the dev hardware per the Phase 6 perf budget (PR-6-16
//! benchmarks this).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::core::gcode::FeatureType;

use super::ir::{BoundingBox, PreviewGeometry};

/// Canonical string form of a [`FeatureType`] for use as a
/// HashMap key when the map crosses the JSON boundary
/// (Tauri returns). JSON requires string keys; serde turns
/// `FeatureType::Other("Custom")` into `{"Other":"Custom"}`
/// which isn't a valid JSON key. Stringifying up-front sidesteps
/// the issue + matches the panel's display label.
fn feature_key(ft: &FeatureType) -> String {
    ft.as_token()
}

/// Per-layer aggregate. One per [`super::ir::LayerRange`], in
/// `layer_index` order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerLayerStats {
    pub layer_index: u32,
    pub z: f32,
    /// Difference from the previous layer's Z. `first_layer_height`
    /// for layer 0 — same as the Z value when no prior layer exists.
    pub layer_height: f32,
    /// Sum of segment durations (extrusion + travel) within the
    /// layer. Excludes retraction time (retracts are ~instant for
    /// the purposes of this visualization).
    pub duration_seconds: f32,
    pub max_speed: f32,
    /// Filament consumed within this layer, mm of filament, keyed
    /// by 0-based tool index.
    pub filament_used_mm: HashMap<u8, f32>,
    /// Time spent in each feature within the layer, seconds.
    /// Keyed by [`feature_key`] (canonical display name) so the
    /// HashMap can round-trip through JSON — see the helper's doc
    /// for why.
    pub feature_breakdown: HashMap<String, f32>,
}

/// Job-level aggregate. Folded from [`PerLayerStats`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FullJobStats {
    pub total_duration_seconds: f32,
    /// See [`PerLayerStats::feature_breakdown`] for the keying.
    pub feature_breakdown: HashMap<String, f32>,
    pub layer_count: u32,
    pub filament_used_mm: HashMap<u8, f32>,
    pub bounding_box: BoundingBox,
    pub layer_heights: HeightStats,
}

/// Per-job layer-height aggregate. `variable` flips on when
/// max-min exceeds `VARIABLE_HEIGHT_TOLERANCE_MM` so the panel can
/// flag variable-layer-height prints.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HeightStats {
    pub min: f32,
    pub max: f32,
    pub variable: bool,
}

/// Threshold (mm) above which a layer-height delta counts as
/// "variable" rather than float noise. 0.005mm ≈ a single
/// micrometer step, well above libslic3r's emitted precision.
const VARIABLE_HEIGHT_TOLERANCE_MM: f32 = 0.005;

/// Walk the geometry's extrusion + travel segments and compute
/// one [`PerLayerStats`] per layer range. Output is in
/// `layer_index` order (the order PR-6-4's build pushes
/// `layer_ranges` in is already monotonic).
pub fn compute_layer_stats(geometry: &PreviewGeometry) -> Vec<PerLayerStats> {
    let mut out = Vec::with_capacity(geometry.layer_ranges.len());

    // Travels overlap layers by source-line order but the IR
    // doesn't index them by layer. Walk travels once + bucket by
    // their per-segment layer index for O(1) lookup per layer.
    let travel_buckets = bucket_travels_by_layer(geometry);

    for (i, range) in geometry.layer_ranges.iter().enumerate() {
        let prev_z = if i == 0 {
            0.0
        } else {
            geometry.layer_ranges[i - 1].z
        };
        let layer_height = (range.z - prev_z).max(0.0);

        let mut stats = PerLayerStats {
            layer_index: range.layer_index,
            z: range.z,
            layer_height,
            duration_seconds: 0.0,
            max_speed: 0.0,
            filament_used_mm: HashMap::new(),
            feature_breakdown: HashMap::new(),
        };

        // Extrusion segments in [segment_start, segment_end).
        for seg in range.segment_start..range.segment_end {
            let i = seg as usize;
            let length = segment_length(&geometry.extrusions.positions, i);
            let speed = geometry.extrusions.speed[i];
            if speed > stats.max_speed {
                stats.max_speed = speed;
            }
            let duration = if speed > 1e-6 { length / speed } else { 0.0 };
            stats.duration_seconds += duration;
            *stats
                .feature_breakdown
                .entry(feature_key(&geometry.extrusions.feature[i]))
                .or_insert(0.0) += duration;
            let extrusion_mm = extrusion_amount_mm(&geometry.extrusions, i);
            *stats
                .filament_used_mm
                .entry(geometry.extrusions.tool[i])
                .or_insert(0.0) += extrusion_mm;
        }

        // Travels for this layer.
        if let Some(travel_idxs) = travel_buckets.get(&range.layer_index) {
            for &i in travel_idxs {
                let length =
                    segment_length(&geometry.travels.positions, i);
                let speed = geometry.travels.speed[i];
                if speed > stats.max_speed {
                    stats.max_speed = speed;
                }
                let duration = if speed > 1e-6 { length / speed } else { 0.0 };
                stats.duration_seconds += duration;
                *stats
                    .feature_breakdown
                    .entry(feature_key(&FeatureType::Travel))
                    .or_insert(0.0) += duration;
            }
        }

        out.push(stats);
    }

    out
}

/// Roll [`PerLayerStats`] up into a [`FullJobStats`].
pub fn compute_job_stats(
    geometry: &PreviewGeometry,
    layer_stats: &[PerLayerStats],
) -> FullJobStats {
    let mut total_duration_seconds = 0.0;
    let mut feature_breakdown: HashMap<String, f32> = HashMap::new();
    let mut filament_used_mm: HashMap<u8, f32> = HashMap::new();
    let mut min_h = f32::INFINITY;
    let mut max_h = f32::NEG_INFINITY;

    for ls in layer_stats {
        total_duration_seconds += ls.duration_seconds;
        for (f, d) in &ls.feature_breakdown {
            *feature_breakdown.entry(f.clone()).or_insert(0.0) += *d;
        }
        for (t, mm) in &ls.filament_used_mm {
            *filament_used_mm.entry(*t).or_insert(0.0) += *mm;
        }
        // Skip layer 0 from height stats — its "layer height" is
        // synthesized from prev_z = 0 and isn't comparable to
        // inter-layer deltas. If the print is only 1 layer it
        // does contribute (clamp below).
        if ls.layer_index > 0 || layer_stats.len() == 1 {
            if ls.layer_height < min_h {
                min_h = ls.layer_height;
            }
            if ls.layer_height > max_h {
                max_h = ls.layer_height;
            }
        }
    }

    if !min_h.is_finite() {
        min_h = 0.0;
    }
    if !max_h.is_finite() {
        max_h = 0.0;
    }

    FullJobStats {
        total_duration_seconds,
        layer_count: layer_stats.len() as u32,
        feature_breakdown,
        filament_used_mm,
        bounding_box: geometry.bounding_box,
        layer_heights: HeightStats {
            min: min_h,
            max: max_h,
            variable: (max_h - min_h) > VARIABLE_HEIGHT_TOLERANCE_MM,
        },
    }
}

/// Flat `layer_index → duration_seconds` array. Consumed by the
/// `ColorMode::LayerTime` encoder (PR-6-5). Length matches the
/// max layer index + 1; missing layer indices fill with 0.
pub fn layer_time_map(layer_stats: &[PerLayerStats]) -> Vec<f32> {
    let max_layer = layer_stats
        .iter()
        .map(|s| s.layer_index)
        .max()
        .unwrap_or(0);
    let mut out = vec![0.0_f32; (max_layer + 1) as usize];
    for ls in layer_stats {
        out[ls.layer_index as usize] = ls.duration_seconds;
    }
    out
}

/// Walk the IR's travel segments once and bucket them by the
/// `layer_index` they were emitted on. Each segment's layer comes
/// from the per-vertex attribute (same value for both vertices).
fn bucket_travels_by_layer(
    geometry: &PreviewGeometry,
) -> HashMap<u32, Vec<usize>> {
    let mut buckets: HashMap<u32, Vec<usize>> = HashMap::new();
    for i in 0..geometry.travels.len() {
        let layer = geometry.travels.layer_index[i * 2] as u32;
        buckets.entry(layer).or_default().push(i);
    }
    buckets
}

fn segment_length(positions: &[f32], seg: usize) -> f32 {
    let base = seg * 6;
    let dx = positions[base + 3] - positions[base];
    let dy = positions[base + 4] - positions[base + 1];
    let dz = positions[base + 5] - positions[base + 2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// `ΔE` in mm of filament for an extrusion segment. The IR
/// doesn't store `delta_e` directly — recover it from
/// `flow × duration / cross_section`. For travels (flow=0) returns
/// 0, which is what filament-used aggregates want.
fn extrusion_amount_mm(segments: &super::ir::SegmentSet, seg: usize) -> f32 {
    let length = segment_length(&segments.positions, seg);
    let speed = segments.speed[seg];
    if speed <= 1e-6 {
        return 0.0;
    }
    let duration = length / speed;
    let flow = segments.flow[seg];
    let vol_mm3 = flow * duration;
    // Inverse of build.rs's flow formula: ΔE = vol / cross-section.
    const CROSS_SECTION: f32 =
        std::f32::consts::PI * (1.75 * 0.5) * (1.75 * 0.5);
    vol_mm3 / CROSS_SECTION
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::preview::build::build_preview;
    use crate::core::gcode::parse_str;

    fn build_stats(src: &str) -> (PreviewGeometry, Vec<PerLayerStats>, FullJobStats) {
        let lines = parse_str(src);
        let geom = build_preview(&lines);
        let ls = compute_layer_stats(&geom);
        let js = compute_job_stats(&geom, &ls);
        (geom, ls, js)
    }

    #[test]
    fn two_layer_print_produces_two_stats_entries() {
        // Real-gcode pattern: `;LAYER_CHANGE` marker emitted BEFORE
        // each layer's extrusion. The parser starts numbering at
        // the first marker (index=0), so the first extrusion lands
        // on layer 0, the second on layer 1.
        let src = ";LAYER_CHANGE\n\
                   ;Z:0.2\n\
                   G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E0.5 F1200\n\
                   ;LAYER_CHANGE\n\
                   ;Z:0.36\n\
                   G1 X10 Y10 E1.0 F1200\n";
        let (_, ls, js) = build_stats(src);
        assert_eq!(ls.len(), 2);
        assert_eq!(ls[0].layer_index, 0);
        assert_eq!(ls[1].layer_index, 1);
        assert_eq!(js.layer_count, 2);
        // Layer-height: layer 0 = 0.2 (synthesized from prev_z=0),
        // layer 1 = 0.36 - 0.2 = 0.16.
        assert!((ls[0].layer_height - 0.2).abs() < 1e-3);
        assert!((ls[1].layer_height - 0.16).abs() < 1e-3);
    }

    #[test]
    fn variable_layer_height_flag_fires_on_inconsistent_heights() {
        // Three layers with consistent first-two heights (0.2) and
        // a thinner third (0.15) → variable.
        let src = ";LAYER_CHANGE\n\
                   ;Z:0.2\n\
                   G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E0.5 F1200\n\
                   ;LAYER_CHANGE\n\
                   ;Z:0.4\n\
                   G1 X10 Y10 E1.0 F1200\n\
                   ;LAYER_CHANGE\n\
                   ;Z:0.55\n\
                   G1 X0 Y10 E1.5 F1200\n";
        let (_, _, js) = build_stats(src);
        // Skipping layer 0 from comparison per the impl rule:
        // layer 1 height = 0.2, layer 2 height = 0.15 → variable.
        assert!(js.layer_heights.variable);
        assert!((js.layer_heights.min - 0.15).abs() < 1e-3);
        assert!((js.layer_heights.max - 0.2).abs() < 1e-3);
    }

    #[test]
    fn job_duration_sums_layer_durations() {
        let src = ";LAYER_CHANGE\n\
                   ;Z:0.2\n\
                   G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E0.5 F1200\n\
                   ;LAYER_CHANGE\n\
                   ;Z:0.4\n\
                   G1 X10 Y10 E1.0 F1200\n";
        let (_, ls, js) = build_stats(src);
        let layer_sum: f32 = ls.iter().map(|l| l.duration_seconds).sum();
        assert!(
            (js.total_duration_seconds - layer_sum).abs() < 1e-4,
            "job total {} should match sum of layer durations {}",
            js.total_duration_seconds,
            layer_sum,
        );
    }

    #[test]
    fn feature_breakdown_sums_to_layer_duration() {
        let src = ";LAYER_CHANGE\n\
                   ;Z:0.2\n\
                   G1 X0 Y0 Z0.2 F1800\n\
                   ;TYPE:External perimeter\n\
                   G1 X10 Y0 E0.5 F1200\n\
                   ;TYPE:Solid infill\n\
                   G1 X10 Y10 E1.0 F1200\n";
        let (_, ls, _) = build_stats(src);
        let bd_sum: f32 = ls[0].feature_breakdown.values().sum();
        assert!(
            (bd_sum - ls[0].duration_seconds).abs() < 1e-3,
            "feature breakdown {} should sum to layer duration {}",
            bd_sum,
            ls[0].duration_seconds,
        );
    }

    #[test]
    fn multi_tool_filament_breakdown() {
        let src = ";LAYER_CHANGE\n\
                   ;Z:0.2\n\
                   G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E1.0 F1200\n\
                   T1\n\
                   G1 X10 Y10 E2.0 F1200\n";
        let (_, ls, js) = build_stats(src);
        // Tool 0 extruded 1.0mm of filament; tool 1 extruded 1.0mm
        // (delta from 1.0 to 2.0).
        assert!(
            (ls[0].filament_used_mm.get(&0).copied().unwrap_or(0.0) - 1.0)
                .abs()
                < 1e-2,
        );
        assert!(
            (ls[0].filament_used_mm.get(&1).copied().unwrap_or(0.0) - 1.0)
                .abs()
                < 1e-2,
        );
        // Job totals match.
        assert!(
            (js.filament_used_mm.get(&0).copied().unwrap_or(0.0) - 1.0).abs()
                < 1e-2,
        );
        assert!(
            (js.filament_used_mm.get(&1).copied().unwrap_or(0.0) - 1.0).abs()
                < 1e-2,
        );
    }

    #[test]
    fn max_speed_tracks_peak() {
        let src = ";LAYER_CHANGE\n\
                   ;Z:0.2\n\
                   G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E0.5 F1200\n\
                   G1 X10 Y10 E1.0 F6000\n";
        let (_, ls, _) = build_stats(src);
        // F=6000 mm/min = 100 mm/s, beats F=1200 (20 mm/s).
        assert!((ls[0].max_speed - 100.0).abs() < 1e-3);
    }

    #[test]
    fn layer_time_map_indexed_by_layer_index() {
        let src = ";LAYER_CHANGE\n\
                   ;Z:0.2\n\
                   G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E0.5 F1200\n\
                   ;LAYER_CHANGE\n\
                   ;Z:0.4\n\
                   G1 X10 Y10 E1.0 F1200\n";
        let (_, ls, _) = build_stats(src);
        let times = layer_time_map(&ls);
        assert_eq!(times.len(), 2);
        assert_eq!(times[0], ls[0].duration_seconds);
        assert_eq!(times[1], ls[1].duration_seconds);
    }

    #[test]
    fn empty_geometry_produces_empty_stats() {
        let src = "";
        let (_, ls, js) = build_stats(src);
        assert!(ls.is_empty());
        assert_eq!(js.layer_count, 0);
        assert_eq!(js.total_duration_seconds, 0.0);
        assert!(js.feature_breakdown.is_empty());
        assert!(!js.layer_heights.variable);
    }

    #[test]
    fn travels_contribute_to_layer_duration() {
        // One extrusion + one travel on the same layer.
        let src = ";LAYER_CHANGE\n\
                   ;Z:0.2\n\
                   G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E0.5 F1200\n\
                   G1 X20 Y10 F6000\n";
        let (_, ls, _) = build_stats(src);
        // Extrusion: 10mm @ 20mm/s = 0.5s.
        // Travel: sqrt(10²+10²) ≈ 14.14mm @ 100mm/s ≈ 0.141s.
        assert!((ls[0].duration_seconds - 0.641).abs() < 0.01);
        // Feature breakdown should include both Travel and the
        // extrusion feature (keyed by canonical display name).
        assert!(ls[0].feature_breakdown.contains_key("Travel"));
    }
}
