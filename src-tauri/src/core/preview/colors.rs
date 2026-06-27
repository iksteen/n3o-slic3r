//! Color-mode encoders.
//!
//! Pure mapping `(SegmentSet, ColorMode, Palette, layer_times) →
//! Vec<f32>` of per-vertex RGB colors. The renderer binds
//! the result as a `color` vertex attribute on the extrusion
//! `LineSegments`; travel + retraction colors are hardcoded
//! frontend-side (flat grey + red dot) and don't flow through here.
//!
//! Two color-space conventions in play:
//!
//! - **Discrete modes** (Feature, Tool) map enum-like inputs to a
//!   small palette. The default palette is **Okabe-Ito** — an
//!   8-color categorical palette designed for color-vision
//!   deficiency (CVD) friendliness, widely cited (Wong, *Points
//!   of view: Color blindness*, Nature Methods 2011).
//! - **Continuous modes** (Speed, Flow, LayerTime) map a scalar
//!   to a position on a colormap. Default is an approximation of
//!   **viridis** — perceptually uniform across deuteranopia,
//!   protanopia, and tritanopia. We use a 5-point control LUT
//!   rather than the full 256-entry map; the lerp between them is
//!   smooth enough for visualization purposes and keeps the table
//!   tiny.
//!
//! All RGB values are floats in `[0.0, 1.0]`, ready for GPU
//! buffer-attribute binding without normalization.

use serde::{Deserialize, Serialize};

use crate::core::gcode::FeatureType;

use super::ir::SegmentSet;

/// Which scalar to drive the per-segment color from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorMode {
    /// Discrete: each `FeatureType` variant → palette entry.
    Feature,
    /// Continuous: speed normalized to the segment set's min/max.
    Speed,
    /// Continuous: volumetric flow likewise.
    Flow,
    /// Continuous: per-layer print duration from stats.
    LayerTime,
    /// Discrete: tool index 0..N → palette entry (modulo 8).
    Tool,
}

/// Palette family for both discrete + continuous modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Palette {
    /// Color-blind-safe default. Discrete: Okabe-Ito 8-color.
    /// Continuous: viridis approximation.
    Default,
    /// Slicer-conventional warm-cool variant. Discrete: rough
    /// match to OrcaSlicer's stock feature palette. Continuous:
    /// blue→cyan→green→yellow→red ramp (the "jet"-style classic).
    /// Less CVD-friendly but more familiar to existing slicer users.
    Classic,
}

/// Encode per-vertex colors for every segment in `segments`.
///
/// Output layout: `positions.len() / 3 * 3` floats = `[r, g, b]`
/// per vertex, 2 vertices per segment, 6 floats per segment.
/// Both vertices of a segment receive the same color (so a future
/// per-vertex gradient mode could swap encoders without changing
/// the renderer).
///
/// `layer_times` is required for `ColorMode::LayerTime` and ignored
/// otherwise. Missing it for LayerTime falls back to the segment's
/// own `layer_index` mapped onto a 0..layer_count ramp — useful
/// for "preview without stats" but not strictly faithful to
/// per-layer duration.
pub fn encode_colors(
    segments: &SegmentSet,
    mode: ColorMode,
    palette: Palette,
    layer_times: Option<&[f32]>,
) -> Vec<f32> {
    let seg_count = segments.len();
    let mut out = Vec::with_capacity(seg_count * 6);

    // Hoist the per-mode lookup out of the inner loop. Continuous
    // modes (Speed / Flow / LayerTime) need a min/max across the
    // whole input array; computing it per-segment is O(N²) and
    // turns a 5 MB G-code's color encode into a 68 s stall.
    // Discrete modes (Feature / Tool) have no precompute step.
    match mode {
        ColorMode::Feature => {
            for i in 0..seg_count {
                let rgb = feature_color(&segments.feature[i], palette);
                out.extend_from_slice(&rgb);
                out.extend_from_slice(&rgb);
            }
        }
        ColorMode::Tool => {
            for i in 0..seg_count {
                let rgb = tool_color(segments.tool[i], palette);
                out.extend_from_slice(&rgb);
                out.extend_from_slice(&rgb);
            }
        }
        ColorMode::Speed => {
            let (lo, hi) = scalar_range(&segments.speed);
            for i in 0..seg_count {
                let rgb = continuous_color(segments.speed[i], lo, hi, palette);
                out.extend_from_slice(&rgb);
                out.extend_from_slice(&rgb);
            }
        }
        ColorMode::Flow => {
            let (lo, hi) = scalar_range(&segments.flow);
            for i in 0..seg_count {
                let rgb = continuous_color(segments.flow[i], lo, hi, palette);
                out.extend_from_slice(&rgb);
                out.extend_from_slice(&rgb);
            }
        }
        ColorMode::LayerTime => {
            let (lo, hi) = match layer_times {
                Some(times) if !times.is_empty() => scalar_range(times),
                _ => (0.0, segments.layer_index.last().copied().unwrap_or(1.0)),
            };
            for i in 0..seg_count {
                let layer = segments.layer_index[i * 2] as usize;
                let t = match layer_times {
                    Some(times) if layer < times.len() => times[layer],
                    _ => segments.layer_index[i * 2], // fallback: layer index
                };
                let rgb = continuous_color(t, lo, hi, palette);
                out.extend_from_slice(&rgb);
                out.extend_from_slice(&rgb);
            }
        }
    }
    out
}

fn scalar_range(values: &[f32]) -> (f32, f32) {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for &v in values {
        if v.is_finite() {
            if v < lo {
                lo = v;
            }
            if v > hi {
                hi = v;
            }
        }
    }
    if !lo.is_finite() || !hi.is_finite() || hi <= lo {
        // Degenerate range (empty, single value, or all identical)
        // — pick (0, 1) so the colormap collapses to its midpoint.
        (0.0, 1.0)
    } else {
        (lo, hi)
    }
}

fn continuous_color(value: f32, lo: f32, hi: f32, palette: Palette) -> [f32; 3] {
    let t = if hi > lo {
        ((value - lo) / (hi - lo)).clamp(0.0, 1.0)
    } else {
        0.5
    };
    match palette {
        Palette::Default => viridis_lerp(t),
        Palette::Classic => jet_lerp(t),
    }
}

/// 5-control-point viridis approximation. Real viridis is a
/// 256-entry LUT generated by matplotlib; the lerp through these
/// stops is visually indistinguishable for line previews and
/// keeps the table sub-10 entries.
const VIRIDIS_STOPS: &[[f32; 3]] = &[
    [0.267, 0.005, 0.329], // dark purple
    [0.281, 0.298, 0.561], // blue-violet
    [0.207, 0.514, 0.557], // teal
    [0.337, 0.731, 0.404], // green
    [0.993, 0.906, 0.144], // yellow
];

fn viridis_lerp(t: f32) -> [f32; 3] {
    palette_lerp(t, VIRIDIS_STOPS)
}

const JET_STOPS: &[[f32; 3]] = &[
    [0.0, 0.0, 0.5], // dark blue
    [0.0, 0.5, 1.0], // blue→cyan
    [0.0, 1.0, 0.0], // green
    [1.0, 1.0, 0.0], // yellow
    [1.0, 0.0, 0.0], // red
];

fn jet_lerp(t: f32) -> [f32; 3] {
    palette_lerp(t, JET_STOPS)
}

/// Lerp `t ∈ [0, 1]` across an ordered set of RGB control points.
/// Each segment of the colormap is sized `1 / (stops - 1)` wide.
fn palette_lerp(t: f32, stops: &[[f32; 3]]) -> [f32; 3] {
    if stops.is_empty() {
        return [0.0, 0.0, 0.0];
    }
    if stops.len() == 1 {
        return stops[0];
    }
    let t = t.clamp(0.0, 1.0);
    let scaled = t * (stops.len() - 1) as f32;
    let lo_idx = (scaled.floor() as usize).min(stops.len() - 2);
    let frac = scaled - lo_idx as f32;
    let lo = stops[lo_idx];
    let hi = stops[lo_idx + 1];
    [
        lo[0] + (hi[0] - lo[0]) * frac,
        lo[1] + (hi[1] - lo[1]) * frac,
        lo[2] + (hi[2] - lo[2]) * frac,
    ]
}

/// Discrete colors for `FeatureType` variants. Default palette
/// uses Okabe-Ito assignments mapped by visual category (warm
/// colors for visible-surface features, cool for hidden).
fn feature_color(feature: &FeatureType, palette: Palette) -> [f32; 3] {
    match palette {
        Palette::Default => match feature {
            FeatureType::ExternalPerimeter => OKABE_VERMILLION,
            FeatureType::Perimeter => OKABE_ORANGE,
            FeatureType::Infill => OKABE_BLUE,
            FeatureType::SolidInfill => OKABE_SKY_BLUE,
            FeatureType::TopSolidInfill => OKABE_YELLOW,
            FeatureType::Bridge => OKABE_REDDISH_PURPLE,
            FeatureType::Support => OKABE_BLUISH_GREEN,
            FeatureType::Skirt | FeatureType::Brim => OKABE_GREY,
            FeatureType::Travel => OKABE_GREY,
            FeatureType::Other(_) => OKABE_GREY,
        },
        Palette::Classic => match feature {
            // Slicer-conventional: warm-cool assignment from
            // OrcaSlicer's default feature legend.
            FeatureType::ExternalPerimeter => [0.91, 0.27, 0.27], // red
            FeatureType::Perimeter => [1.0, 0.55, 0.0],           // orange
            FeatureType::Infill => [0.13, 0.55, 0.85],            // blue
            FeatureType::SolidInfill => [0.35, 0.78, 0.96],       // light blue
            FeatureType::TopSolidInfill => [1.0, 0.9, 0.2],       // yellow
            FeatureType::Bridge => [0.6, 0.4, 0.8],               // purple
            FeatureType::Support => [0.4, 0.7, 0.4],              // green
            FeatureType::Skirt | FeatureType::Brim => [0.6, 0.6, 0.6],
            FeatureType::Travel => [0.5, 0.5, 0.5],
            FeatureType::Other(_) => [0.5, 0.5, 0.5],
        },
    }
}

/// Discrete colors for tool indices. Tool 0..7 cycle through the
/// Okabe-Ito palette; tool ≥ 8 wraps via modulo so a 16-tool
/// printer (none exists but Phase 7 might) doesn't crash.
fn tool_color(tool: u8, palette: Palette) -> [f32; 3] {
    let cycle = (tool as usize) % 8;
    match palette {
        Palette::Default => OKABE_TOOL_CYCLE[cycle],
        Palette::Classic => CLASSIC_TOOL_CYCLE[cycle],
    }
}

// ─── Okabe-Ito 8-color CVD-safe palette ──────────────────────
// Wong, B. *Points of view: Color blindness.* Nature Methods 8,
// 441 (2011). https://doi.org/10.1038/nmeth.1618
const OKABE_GREY: [f32; 3] = [0.4, 0.4, 0.4];
const OKABE_ORANGE: [f32; 3] = [0.902, 0.624, 0.0];
const OKABE_SKY_BLUE: [f32; 3] = [0.337, 0.706, 0.914];
const OKABE_BLUISH_GREEN: [f32; 3] = [0.0, 0.620, 0.451];
const OKABE_YELLOW: [f32; 3] = [0.941, 0.894, 0.259];
const OKABE_BLUE: [f32; 3] = [0.0, 0.447, 0.698];
const OKABE_VERMILLION: [f32; 3] = [0.835, 0.369, 0.0];
const OKABE_REDDISH_PURPLE: [f32; 3] = [0.800, 0.475, 0.655];

const OKABE_TOOL_CYCLE: [[f32; 3]; 8] = [
    OKABE_VERMILLION,
    OKABE_SKY_BLUE,
    OKABE_BLUISH_GREEN,
    OKABE_ORANGE,
    OKABE_BLUE,
    OKABE_REDDISH_PURPLE,
    OKABE_YELLOW,
    OKABE_GREY,
];

// ─── Classic palette tool cycle ──────────────────────────────
const CLASSIC_TOOL_CYCLE: [[f32; 3]; 8] = [
    [0.91, 0.27, 0.27], // red
    [0.13, 0.55, 0.85], // blue
    [0.4, 0.7, 0.4],    // green
    [1.0, 0.55, 0.0],   // orange
    [0.6, 0.4, 0.8],    // purple
    [1.0, 0.9, 0.2],    // yellow
    [0.35, 0.78, 0.96], // light blue
    [0.6, 0.6, 0.6],    // grey
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::preview::ir::Segment;

    fn fixture_segments() -> SegmentSet {
        let mut s = SegmentSet::default();
        s.push(Segment {
            start: [0.0, 0.0, 0.2],
            end: [10.0, 0.0, 0.2],
            layer: 0,
            feature: FeatureType::ExternalPerimeter,
            speed: 50.0,
            flow: 5.0,
            extrusion_mm: 0.0,
            tool: 0,
            source_line: 0,
            width: 0.45,
            height: 0.2,
        });
        s.push(Segment {
            start: [10.0, 0.0, 0.2],
            end: [10.0, 10.0, 0.2],
            layer: 0,
            feature: FeatureType::Infill,
            speed: 100.0,
            flow: 10.0,
            extrusion_mm: 0.0,
            tool: 1,
            source_line: 1,
            width: 0.45,
            height: 0.2,
        });
        s
    }

    #[test]
    fn output_length_is_six_floats_per_segment() {
        let s = fixture_segments();
        let colors = encode_colors(&s, ColorMode::Feature, Palette::Default, None);
        assert_eq!(colors.len(), s.len() * 6);
    }

    #[test]
    fn feature_mode_is_deterministic() {
        let s = fixture_segments();
        let a = encode_colors(&s, ColorMode::Feature, Palette::Default, None);
        let b = encode_colors(&s, ColorMode::Feature, Palette::Default, None);
        assert_eq!(a, b);
        // External perimeter (segment 0) ≠ Infill (segment 1).
        assert_ne!(&a[0..3], &a[6..9]);
    }

    #[test]
    fn tool_indices_cycle_modulo_eight() {
        let mut s = SegmentSet::default();
        for tool in [1u8, 9, 17] {
            // 1, 9, 17 all map to position 1 of the cycle.
            s.push(Segment {
                start: [0.0; 3],
                end: [1.0, 0.0, 0.0],
                layer: 0,
                feature: FeatureType::Perimeter,
                speed: 50.0,
                flow: 5.0,
                extrusion_mm: 0.0,
                tool,
                source_line: 0,
                width: 0.45,
                height: 0.2,
            });
        }
        let c = encode_colors(&s, ColorMode::Tool, Palette::Default, None);
        // All three segments should hit the same palette entry.
        assert_eq!(&c[0..3], &c[6..9]);
        assert_eq!(&c[0..3], &c[12..15]);
    }

    #[test]
    fn continuous_mode_normalizes_to_range() {
        let s = fixture_segments();
        let c = encode_colors(&s, ColorMode::Speed, Palette::Default, None);
        // Segment 0 (speed=50, the min) → viridis[0] (dark purple).
        let lowest = VIRIDIS_STOPS[0];
        for axis in 0..3 {
            assert!(
                (c[axis] - lowest[axis]).abs() < 1e-3,
                "axis {axis}: got {} expected {}",
                c[axis],
                lowest[axis],
            );
        }
        // Segment 1 (speed=100, the max) → viridis[last] (yellow).
        let highest = VIRIDIS_STOPS[VIRIDIS_STOPS.len() - 1];
        for axis in 0..3 {
            assert!(
                (c[6 + axis] - highest[axis]).abs() < 1e-3,
                "axis {axis}: got {} expected {}",
                c[6 + axis],
                highest[axis],
            );
        }
    }

    #[test]
    fn layer_time_uses_provided_times_when_present() {
        let mut s = SegmentSet::default();
        // Two segments, two different layers.
        s.push(Segment {
            start: [0.0; 3],
            end: [1.0, 0.0, 0.0],
            layer: 0,
            feature: FeatureType::Perimeter,
            speed: 50.0,
            flow: 5.0,
            extrusion_mm: 0.0,
            tool: 0,
            source_line: 0,
            width: 0.45,
            height: 0.2,
        });
        s.push(Segment {
            start: [0.0; 3],
            end: [1.0, 0.0, 0.0],
            layer: 1,
            feature: FeatureType::Perimeter,
            speed: 50.0,
            flow: 5.0,
            extrusion_mm: 0.0,
            tool: 0,
            source_line: 1,
            width: 0.45,
            height: 0.2,
        });
        // Layer 0 fast, layer 1 slow.
        let layer_times = vec![10.0_f32, 120.0_f32];
        let c = encode_colors(
            &s,
            ColorMode::LayerTime,
            Palette::Default,
            Some(&layer_times),
        );
        // Layer 0 → viridis[0] (low end).
        assert!((c[0] - VIRIDIS_STOPS[0][0]).abs() < 1e-3);
        // Layer 1 → viridis[last] (high end).
        let n = VIRIDIS_STOPS.len() - 1;
        assert!((c[6] - VIRIDIS_STOPS[n][0]).abs() < 1e-3);
    }

    #[test]
    fn degenerate_range_collapses_to_palette_midpoint() {
        // All segments same speed → range is degenerate, lerp t=0.5.
        let mut s = SegmentSet::default();
        for _ in 0..3 {
            s.push(Segment {
                start: [0.0; 3],
                end: [1.0, 0.0, 0.0],
                layer: 0,
                feature: FeatureType::Perimeter,
                speed: 60.0,
                flow: 5.0,
                extrusion_mm: 0.0,
                tool: 0,
                source_line: 0,
                width: 0.45,
                height: 0.2,
            });
        }
        let c = encode_colors(&s, ColorMode::Speed, Palette::Default, None);
        // All three segments should be the same color (midpoint).
        assert_eq!(&c[0..3], &c[6..9]);
        assert_eq!(&c[0..3], &c[12..15]);
    }

    #[test]
    fn empty_segment_set_produces_empty_output() {
        let s = SegmentSet::default();
        let c = encode_colors(&s, ColorMode::Feature, Palette::Default, None);
        assert!(c.is_empty());
    }

    #[test]
    fn classic_palette_is_distinct_from_default() {
        let s = fixture_segments();
        let d = encode_colors(&s, ColorMode::Feature, Palette::Default, None);
        let c = encode_colors(&s, ColorMode::Feature, Palette::Classic, None);
        assert_ne!(d, c, "palette switch produces different output");
    }

    #[test]
    fn viridis_lerp_endpoints_match_lut() {
        assert_eq!(viridis_lerp(0.0), VIRIDIS_STOPS[0]);
        assert_eq!(viridis_lerp(1.0), VIRIDIS_STOPS[VIRIDIS_STOPS.len() - 1]);
    }
}
