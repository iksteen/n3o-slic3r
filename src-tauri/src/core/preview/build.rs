//! Preview IR build — walk a typed G-code stream into renderable
//! segments.
//!
//! State machine over `&[Line]`:
//!
//! - Track current position (X/Y/Z/E), feedrate, tool, feature.
//!   Per-axis `None` semantics from [`crate::core::gcode::Position`]
//!   are preserved (missing axis keeps the previous value).
//! - On [`crate::core::gcode::Line::Move`]: classify as extrusion
//!   (E increases) / travel (XY changes without E increase) /
//!   retraction (E decreases without XY motion) / no-op. Push to
//!   the matching `SegmentSet` or `retractions` vec.
//! - On [`crate::core::gcode::Line::LayerChange`]: close the
//!   current `LayerRange` (set its `segment_end`) and open the
//!   next one. The layer index threaded into extrusion segments
//!   updates from here on.
//! - On [`crate::core::gcode::Line::ToolChange`]: bump the tool
//!   index.
//! - On a `;TYPE:` `Comment` (semantic `FeatureType`): update the
//!   current feature classification.
//!
//! Determinism: segment order matches G-code command order;
//! `source_line` is monotonically increasing within each
//! `SegmentSet`. The hover-inspection raycast depends
//! on this.

use crate::core::gcode::{FeatureType, Line, SemanticComment};

use super::ir::{BoundingBox, LayerRange, PreviewGeometry, RetractionMarker, Segment};

/// Filament diameter assumed when converting `ΔE` (mm of filament
/// off the spool) to volumetric flow (mm³). 1.75mm is the
/// near-universal default; the actual value lives in the cascade's
/// `filament_diameter` setting + may be parsed from the gcode
/// header. Wiring that in is a Phase 6 polish — the color encoder
/// uses flow for relative coloring, not absolute correctness, so
/// the hardcoded default is fine for MVP.
const FILAMENT_DIAMETER_MM: f32 = 1.75;
const FILAMENT_CROSS_SECTION_MM2: f32 =
    std::f32::consts::PI * (FILAMENT_DIAMETER_MM * 0.5) * (FILAMENT_DIAMETER_MM * 0.5);

/// State carried through the walk. Initial position is at the
/// origin with no extrusion + no feedrate — `G28` would establish
/// this in real printer flow, but we don't depend on seeing it.
#[derive(Debug, Clone)]
struct WalkState {
    x: f32,
    y: f32,
    z: f32,
    e: f32,
    /// Most recently issued feed rate in mm/min (libslic3r's
    /// convention). Converted to mm/s when emitted as segment
    /// speed. 0 → unknown speed; treated as 0 mm/s in the IR.
    feedrate: f32,
    tool: u8,
    feature: FeatureType,
    layer_index: u32,
    layer_z: f32,
}

impl Default for WalkState {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            e: 0.0,
            feedrate: 0.0,
            tool: 0,
            // `Travel` is the safe pre-first-`;TYPE:` default —
            // header skirt/brim moves before any feature marker
            // arrive land as travels at the GPU, which is
            // visually quiet (and they ARE pre-print priming
            // anyway).
            feature: FeatureType::Travel,
            layer_index: 0,
            layer_z: 0.0,
        }
    }
}

/// Walk a typed G-code stream into a renderable [`PreviewGeometry`].
///
/// `lines` is the parser's output (from [`crate::core::gcode::parse_str`]
/// or `parse_lines`). The 0-based line index becomes the
/// `source_line` back-reference on each emitted segment.
pub fn build_preview(lines: &[Line]) -> PreviewGeometry {
    let mut geom = PreviewGeometry::default();
    let mut state = WalkState::default();

    // Current open layer range. Started on the first emitted
    // extrusion segment; closed when a new LayerChange fires or
    // the walk ends.
    let mut current_layer_range: Option<LayerRange> = None;

    for (idx, line) in lines.iter().enumerate() {
        let line_idx = idx as u32;
        match line {
            Line::Move(mv) => {
                let prev = (state.x, state.y, state.z);
                let target_x = mv.target.x.unwrap_or(state.x);
                let target_y = mv.target.y.unwrap_or(state.y);
                let target_z = mv.target.z.unwrap_or(state.z);
                let target_e = mv.target.e.unwrap_or(state.e);

                if let Some(f) = mv.feedrate {
                    state.feedrate = f as f32;
                }

                let delta_e = target_e - state.e;
                let xy_moved =
                    (target_x - state.x).abs() > 1e-6 || (target_y - state.y).abs() > 1e-6;
                let z_moved = (target_z - state.z).abs() > 1e-6;

                let start = [prev.0, prev.1, prev.2];
                let end = [target_x, target_y, target_z];

                // Classification:
                //   delta_e > 0  + (xy_moved || z_moved) → extrusion
                //   delta_e > 0  + no xy/z motion         → in-place extrude (rare, skip)
                //   delta_e < 0  + no xy_moved            → retraction (no segment)
                //   delta_e < 0  + xy_moved               → travel + retraction marker at start
                //   delta_e == 0 + xy/z motion            → travel
                //   delta_e == 0 + no motion              → nothing
                if delta_e > 1e-6 && (xy_moved || z_moved) {
                    // Extrusion segment.
                    let length = euclidean(start, end);
                    let speed_mm_s = state.feedrate / 60.0;
                    let flow_mm3_s = if length > 1e-6 && speed_mm_s > 1e-6 {
                        let duration = length / speed_mm_s;
                        let vol_mm3 = delta_e * FILAMENT_CROSS_SECTION_MM2;
                        vol_mm3 / duration
                    } else {
                        0.0
                    };
                    open_or_continue_layer(
                        &mut current_layer_range,
                        &mut geom.layer_ranges,
                        state.layer_index,
                        state.layer_z,
                        geom.extrusions.len() as u32,
                    );
                    geom.extrusions.push(Segment {
                        start,
                        end,
                        layer: state.layer_index,
                        feature: state.feature.clone(),
                        speed: speed_mm_s,
                        flow: flow_mm3_s,
                        tool: state.tool,
                        source_line: line_idx,
                    });
                    geom.bounding_box.extend(start);
                    geom.bounding_box.extend(end);
                } else if delta_e < -1e-6 && !xy_moved {
                    // Pure retraction.
                    geom.retractions.push(RetractionMarker {
                        position: start,
                        layer_index: state.layer_index,
                        amount_mm: -delta_e,
                    });
                } else if xy_moved || z_moved {
                    // Travel (covers both delta_e == 0 with motion
                    // AND delta_e < 0 with motion). For the
                    // retraction-with-motion case, also push a
                    // retraction marker at the start.
                    let speed_mm_s = state.feedrate / 60.0;
                    if delta_e < -1e-6 {
                        geom.retractions.push(RetractionMarker {
                            position: start,
                            layer_index: state.layer_index,
                            amount_mm: -delta_e,
                        });
                    }
                    geom.travels.push(Segment {
                        start,
                        end,
                        layer: state.layer_index,
                        feature: FeatureType::Travel,
                        speed: speed_mm_s,
                        flow: 0.0,
                        tool: state.tool,
                        source_line: line_idx,
                    });
                }
                // Else: no XY/Z motion + no E change → genuine
                // no-op, ignore.

                state.x = target_x;
                state.y = target_y;
                state.z = target_z;
                state.e = target_e;
            }

            Line::LayerChange(lc) => {
                // Close the current layer range (if any) by
                // setting its `segment_end` to the current
                // extrusion count.
                if let Some(mut r) = current_layer_range.take() {
                    r.segment_end = geom.extrusions.len() as u32;
                    geom.layer_ranges.push(r);
                }
                state.layer_index = lc.index;
                if let Some(z) = lc.z {
                    state.layer_z = z;
                    state.z = z; // some flavors track Z via comment-only
                }
                // Start the new layer range lazily — `open_or_
                // continue_layer` is called from the extrusion
                // branch so we don't push empty ranges for the
                // pre-first-extrusion layer.
            }

            Line::ToolChange(tc) => {
                state.tool = tc.extruder;
            }

            Line::Comment(c) => {
                if let Some(SemanticComment::FeatureType(ft)) = &c.semantic {
                    state.feature = ft.clone();
                } else if let Some(SemanticComment::Z(z)) = &c.semantic {
                    // `;Z:0.2` without a paired LayerChange (the
                    // parser may or may not synthesize one — we
                    // treat the Z comment as authoritative for
                    // future extrusion segments).
                    state.layer_z = *z;
                    state.z = *z;
                }
            }

            Line::Other(_) => {
                // M-commands, blank lines, unknown G-codes — no
                // effect on the preview IR.
            }
        }
    }

    // Close the final open layer range.
    if let Some(mut r) = current_layer_range.take() {
        r.segment_end = geom.extrusions.len() as u32;
        geom.layer_ranges.push(r);
    }

    // Zero-out bbox when empty so callers don't see ±∞.
    if geom.bounding_box.is_empty() {
        geom.bounding_box = BoundingBox {
            min: [0.0; 3],
            max: [0.0; 3],
        };
    }

    geom
}

/// Open a new LayerRange when the first extrusion of a layer is
/// emitted, or no-op when one's already open for `layer_index`.
/// Keeps `segment_start` accurate (matches the extrusion index
/// of the first segment in the layer).
fn open_or_continue_layer(
    current: &mut Option<LayerRange>,
    completed: &mut Vec<LayerRange>,
    layer_index: u32,
    layer_z: f32,
    segment_index_now: u32,
) {
    if let Some(r) = current {
        if r.layer_index == layer_index {
            return; // already open for this layer
        }
        // Layer index changed mid-stream without a LayerChange
        // event (shouldn't happen in well-formed gcode but
        // defensive). Close + reopen.
        let mut closed = current.take().unwrap();
        closed.segment_end = segment_index_now;
        completed.push(closed);
    }
    *current = Some(LayerRange {
        layer_index,
        z: layer_z,
        segment_start: segment_index_now,
        segment_end: segment_index_now,
    });
}

fn euclidean(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let dz = b[2] - a[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::gcode::parse_str;

    fn build(src: &str) -> PreviewGeometry {
        let lines = parse_str(src);
        build_preview(&lines)
    }

    #[test]
    fn empty_input_produces_empty_geometry() {
        let g = build("");
        assert!(g.extrusions.is_empty());
        assert!(g.travels.is_empty());
        assert!(g.retractions.is_empty());
        assert!(g.layer_ranges.is_empty());
        // BBox should be zero-ed (not ±∞).
        assert_eq!(g.bounding_box.min, [0.0; 3]);
        assert_eq!(g.bounding_box.max, [0.0; 3]);
    }

    #[test]
    fn single_extrusion_segment_extracted() {
        // Move to start (travel), then extrude to a second point.
        let src = "G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E0.5 F1200\n";
        let g = build(src);
        assert_eq!(g.travels.len(), 1, "first move with no E is a travel");
        assert_eq!(g.extrusions.len(), 1, "second move extrudes");
        // Extrusion positions: start = (0,0,0.2), end = (10,0,0.2).
        assert_eq!(
            &g.extrusions.positions[..6],
            &[0.0, 0.0, 0.2, 10.0, 0.0, 0.2][..]
        );
        // Speed: 1200 mm/min = 20 mm/s.
        assert!((g.extrusions.speed[0] - 20.0).abs() < 1e-3);
    }

    #[test]
    fn travel_extrusion_classification_per_e_delta() {
        // - move with no E → travel
        // - move with E increasing + XY → extrusion
        // - move with E decreasing + no XY → retraction (no segment)
        let src = "G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E0.5 F1200\n\
                   G1 E0.0 F1800\n\
                   G1 X20 Y0 F1500\n";
        let g = build(src);
        assert_eq!(g.extrusions.len(), 1);
        assert_eq!(g.travels.len(), 2, "first + last moves are travels");
        assert_eq!(
            g.retractions.len(),
            1,
            "E=0 from E=0.5 with no XY → retract"
        );
        assert!((g.retractions[0].amount_mm - 0.5).abs() < 1e-3);
    }

    #[test]
    fn layer_ranges_close_on_layer_change() {
        let src = "G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E0.5 F1200\n\
                   ;LAYER_CHANGE\n\
                   ;Z:0.4\n\
                   G1 X10 Y10 E1.0 F1200\n\
                   G1 X0 Y10 E1.5 F1200\n";
        let g = build(src);
        assert_eq!(g.extrusions.len(), 3, "1 on layer 0 + 2 on layer 1");
        assert_eq!(g.layer_ranges.len(), 2);
        assert_eq!(g.layer_ranges[0].segment_start, 0);
        assert_eq!(g.layer_ranges[0].segment_end, 1);
        assert_eq!(g.layer_ranges[1].segment_start, 1);
        assert_eq!(g.layer_ranges[1].segment_end, 3);
        // Layer 1 picks up Z=0.4 from the marker.
        assert!((g.layer_ranges[1].z - 0.4).abs() < 1e-3);
    }

    #[test]
    fn feature_type_threads_through_extrusions() {
        let src = "G1 X0 Y0 Z0.2 F1800\n\
                   ;TYPE:External perimeter\n\
                   G1 X10 Y0 E0.5 F1200\n\
                   ;TYPE:Solid infill\n\
                   G1 X10 Y10 E1.0 F1200\n";
        let g = build(src);
        assert_eq!(g.extrusions.len(), 2);
        assert_eq!(g.extrusions.feature[0], FeatureType::ExternalPerimeter);
        assert_eq!(g.extrusions.feature[1], FeatureType::SolidInfill);
    }

    #[test]
    fn tool_index_updates_on_tool_change() {
        let src = "G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E0.5 F1200\n\
                   T1\n\
                   G1 X10 Y10 E1.0 F1200\n\
                   T0\n\
                   G1 X0 Y10 E1.5 F1200\n";
        let g = build(src);
        assert_eq!(g.extrusions.tool, vec![0, 1, 0]);
    }

    #[test]
    fn bbox_covers_extrusion_endpoints_only() {
        // Travel sweep at the back of the bed shouldn't pull bbox.
        let src = "G1 X100 Y100 Z0.2 F1800\n\
                   G1 X0 Y0 F1800\n\
                   G1 X10 Y10 E0.5 F1200\n";
        let g = build(src);
        // Only the third move is an extrusion (start=(0,0,0.2), end=(10,10,0.2)).
        assert_eq!(g.bounding_box.min, [0.0, 0.0, 0.2]);
        assert_eq!(g.bounding_box.max, [10.0, 10.0, 0.2]);
    }

    #[test]
    fn source_line_indices_match_gcode_order() {
        let src = "G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E0.5 F1200\n\
                   G1 X10 Y10 E1.0 F1200\n";
        let g = build(src);
        // Travel at index 0, extrusions at indices 1 and 2.
        assert_eq!(g.travels.source_line, vec![0]);
        assert_eq!(g.extrusions.source_line, vec![1, 2]);
    }

    #[test]
    fn missing_axes_keep_previous_value() {
        // Second move omits Z — must keep Z=0.2 from the first move.
        let src = "G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E0.5 F1200\n";
        let g = build(src);
        // End Z of the extrusion should be 0.2, not 0.
        assert!((g.extrusions.positions[5] - 0.2).abs() < 1e-3);
    }

    #[test]
    fn flow_rate_derives_from_extrusion_and_length() {
        // 10mm linear extrusion at 20 mm/s with 0.5mm of filament:
        //   duration = 10 / 20 = 0.5s
        //   volume   = 0.5 * π * (1.75/2)² ≈ 1.203 mm³
        //   flow     = 1.203 / 0.5         ≈ 2.405 mm³/s
        let src = "G1 X0 Y0 Z0.2 F1800\n\
                   G1 X10 Y0 E0.5 F1200\n";
        let g = build(src);
        let expected = 0.5 * std::f32::consts::PI * (1.75_f32 / 2.0).powi(2) / 0.5;
        assert!(
            (g.extrusions.flow[0] - expected).abs() < 1e-3,
            "flow {} expected {expected}",
            g.extrusions.flow[0],
        );
    }
}
