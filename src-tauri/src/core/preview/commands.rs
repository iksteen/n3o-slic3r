//! Tauri command surface for the preview pipeline.
//!
//! These commands cover the preview's load + inspection lifecycle.
//! Rendering is owned by [`crate::toolpath_render`], which pulls the
//! IR from the registry by handle and draws it as instanced tubes —
//! geometry never crosses the IPC bridge.
//!
//! - [`preview_load`] — parse + build IR + compute stats. Returns
//!   a [`PreviewLoadResponse`] with the handle, header metadata,
//!   and layer/segment counts the frontend needs.
//! - [`preview_layer_stats`] — per-layer stats as JSON (small;
//!   one row per layer).
//! - [`preview_segment_detail`] — hover-inspection lookup for one
//!   segment index. Returns the source gcode line + position +
//!   speed + feature + layer.
//! - [`preview_drop`] — free the handle's preview. Required
//!   because 50MB gcode → ~250MB resident memory.
//!
//! Concurrency: Tauri command handlers run on Tauri's threadpool
//! (off the main thread). Parse + IR build for a 50MB gcode
//! takes ~2-3s; that's a single command call blocking that
//! thread for that long. Long-form load progress events are a
//! Phase 6 polish — for MVP the frontend shows a spinner.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::core::gcode::{parse_str, FeatureType, HeaderMetadata, Line, Move};

use super::build::build_preview;
use super::ir::BoundingBox;
use super::registry::{LoadedPreview, PreviewHandle, PreviewRegistry};
use super::stats::{compute_job_stats, compute_layer_stats, FullJobStats, PerLayerStats};

/// What `preview_load` returns. The frontend uses `layer_count`
/// to clamp slider bounds, `extrusion_count` / `travel_count` /
/// `retraction_count` to size buffer offsets, and `job_stats` to
/// populate the full-job panel without a follow-up
/// invoke.
#[derive(Debug, Clone, Serialize)]
pub struct PreviewLoadResponse {
    pub handle: PreviewHandle,
    pub header: HeaderMetadata,
    pub layer_count: u32,
    pub extrusion_count: u32,
    pub travel_count: u32,
    pub retraction_count: u32,
    pub bounding_box: BoundingBox,
    pub job_stats: FullJobStats,
}

/// Parse the gcode at `path`, build the preview IR, compute
/// stats, and register the result under a fresh
/// [`PreviewHandle`].
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub fn preview_load(
    path: String,
    registry: State<Arc<PreviewRegistry>>,
) -> Result<PreviewLoadResponse, String> {
    let src = std::fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))?;
    Ok(register_preview(&registry, PathBuf::from(&path), &src))
}

/// Drag-drop loader for Bambu `.gcode.3mf` containers.
///
/// Unpacks the 3MF, extracts the first plate's embedded G-code,
/// runs the same parse+IR+stats pipeline as [`preview_load`], and
/// returns the standard [`PreviewLoadResponse`] alongside the
/// container's plate count, the first plate's pre-baked
/// [`SlicedPlateMetadata`] (estimated time, filament use, AMS
/// bindings — surfaced in the stats panel), and the first plate's
/// optional thumbnail PNG.
///
/// Multi-plate behavior: MVP loads plate 1. The `plate_count`
/// field lets the frontend show a "Plate 1 of N" badge so the
/// user knows the other plates exist; a plate picker is deferred
/// per the index's open question.
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub fn preview_load_gcode_3mf(
    path: String,
    registry: State<Arc<PreviewRegistry>>,
) -> Result<PreviewLoadGcode3mfResponse, String> {
    let read = crate::core::threemf::read_sliced_3mf(std::path::Path::new(&path))
        .map_err(|e| format!("read {path}: {e}"))?;
    if read.plates.is_empty() {
        return Err(format!("{path}: no Metadata/plate_<N>.gcode entries found"));
    }
    let plate_count = read.plates.len() as u32;
    if plate_count > 1 {
        tracing::warn!(
            path = %path,
            plate_count,
            "preview_load_gcode_3mf: multi-plate .gcode.3mf; MVP loads plate 1",
        );
    }
    let plate = read.plates.into_iter().next().expect("len > 0 checked");
    let src = String::from_utf8(plate.gcode).map_err(|e| {
        format!(
            "{path}: plate {} gcode is not utf-8 ({e}); preview pipeline needs text",
            plate.plate_id,
        )
    })?;
    let preview = register_preview(&registry, PathBuf::from(&path), &src);
    Ok(PreviewLoadGcode3mfResponse {
        preview,
        plate_count,
        plate_metadata: plate.metadata,
        thumbnail_png: plate.thumbnail_png,
    })
}

/// What [`preview_load_gcode_3mf`] returns. The `preview` field is
/// the same shape `preview_load` produces (so the frontend's load
/// handlers share a path); the rest is `.gcode.3mf`-specific
/// surface (metadata + thumbnail + multi-plate hint).
#[derive(Debug, Clone, Serialize)]
pub struct PreviewLoadGcode3mfResponse {
    pub preview: PreviewLoadResponse,
    pub plate_count: u32,
    pub plate_metadata: Option<crate::core::threemf::SlicedPlateMetadata>,
    /// Bytes of the embedded PNG, ready for the frontend to wrap
    /// in a `Blob` for `<img>` display. `None` when the file
    /// omitted a thumbnail.
    pub thumbnail_png: Option<Vec<u8>>,
}

/// Shared parse→IR→stats→register path. Source for both
/// [`preview_load`] (raw `.gcode` file) and
/// [`preview_load_gcode_3mf`] (gcode body extracted from a 3MF
/// container). The caller picks how to obtain `src`; this helper
/// owns the wiring so the two commands can't drift on, e.g.,
/// header-vs-body parser choice.
fn register_preview(
    registry: &PreviewRegistry,
    source_path: PathBuf,
    src: &str,
) -> PreviewLoadResponse {
    let header = crate::core::gcode::parse_all_metadata(src.as_bytes());
    let lines = parse_str(src);
    let geometry = build_preview(&lines);
    let layer_stats = compute_layer_stats(&geometry);
    let job_stats = compute_job_stats(&geometry, &layer_stats);

    let layer_count = job_stats.layer_count;
    let extrusion_count = geometry.extrusions.len() as u32;
    let travel_count = geometry.travels.len() as u32;
    let retraction_count = geometry.retractions.len() as u32;
    let bounding_box = geometry.bounding_box;

    let handle = registry.alloc_id();
    registry.insert(
        handle,
        LoadedPreview {
            source_path,
            header: header.clone(),
            geometry,
            layer_stats,
            job_stats: job_stats.clone(),
            lines,
        },
    );

    PreviewLoadResponse {
        handle,
        header,
        layer_count,
        extrusion_count,
        travel_count,
        retraction_count,
        bounding_box,
        job_stats,
    }
}

/// Return the per-layer stats as JSON. Small payload (~one row
/// per layer × ~200 bytes/row) — JSON is fine here, no binary
/// optimization required.
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub fn preview_layer_stats(
    handle: PreviewHandle,
    registry: State<Arc<PreviewRegistry>>,
) -> Result<Vec<PerLayerStats>, String> {
    registry
        .with(handle, |p| p.layer_stats.clone())
        .ok_or_else(|| format!("unknown preview handle {}", handle.0))
}

/// Hover-inspection lookup. Given a segment index into
/// the extrusions array (the raycast result), return the original
/// gcode line text + position + speed + feature + layer + tool +
/// extrusion volume.
///
/// `segment_index` is the index into the extrusions array (not
/// the travels array — travels don't support hover for now;
/// they're visual context, not inspectable geometry).
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub fn preview_segment_detail(
    handle: PreviewHandle,
    segment_index: u32,
    registry: State<Arc<PreviewRegistry>>,
) -> Result<SegmentDetail, String> {
    registry
        .with(handle, |p| {
            let i = segment_index as usize;
            if i >= p.geometry.extrusions.len() {
                return Err(format!(
                    "segment index {segment_index} out of range \
                     (extrusion count {})",
                    p.geometry.extrusions.len(),
                ));
            }
            let segs = &p.geometry.extrusions;
            let base = i * 6;
            let start = [
                segs.positions[base],
                segs.positions[base + 1],
                segs.positions[base + 2],
            ];
            let end = [
                segs.positions[base + 3],
                segs.positions[base + 4],
                segs.positions[base + 5],
            ];
            let source_line = segs.source_line[i] as usize;
            let source_line_text = p
                .lines
                .get(source_line)
                .map(line_raw_text)
                .unwrap_or_default();
            let length = euclidean(start, end);
            let extrusion_mm = if segs.speed[i] > 1e-6 {
                let duration = length / segs.speed[i];
                let vol_mm3 = segs.flow[i] * duration;
                const CROSS_SECTION: f32 = std::f32::consts::PI * (1.75 * 0.5) * (1.75 * 0.5);
                vol_mm3 / CROSS_SECTION
            } else {
                0.0
            };
            Ok(SegmentDetail {
                source_line_text,
                start,
                end,
                speed: segs.speed[i],
                feature: segs.feature[i].clone(),
                layer_index: segs.layer_index[i * 2] as u32,
                tool: segs.tool[i],
                extrusion_mm,
                width: segs.width[i],
                height: segs.height[i],
            })
        })
        .ok_or_else(|| format!("unknown preview handle {}", handle.0))?
}

/// Drop the preview at `handle`. Frees the line stream + IR *and* the
/// renderer's cached GPU buffers for that handle (one lifecycle, so a
/// switched-away preview leaks neither). Silent no-op when the handle
/// wasn't registered (e.g. dropped twice).
#[tauri::command]
#[tracing::instrument(skip(registry, toolpath))]
pub fn preview_drop(
    handle: PreviewHandle,
    registry: State<Arc<PreviewRegistry>>,
    toolpath: State<crate::toolpath_render::ToolpathState>,
) -> Result<(), String> {
    registry.remove(handle);
    if let Some(r) = toolpath.0.lock().unwrap().as_mut() {
        r.drop_handle(handle);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentDetail {
    /// Source gcode line as the parser saw it, line ending
    /// stripped. Used by the hover tooltip for
    /// `G1 X120.34 Y84.21 E0.0341 F1800`-style display.
    pub source_line_text: String,
    pub start: [f32; 3],
    pub end: [f32; 3],
    /// mm/s.
    pub speed: f32,
    pub feature: FeatureType,
    pub layer_index: u32,
    pub tool: u8,
    pub extrusion_mm: f32,
    /// Extrusion line width / layer height (mm) for the hover tooltip.
    pub width: f32,
    pub height: f32,
}

fn line_raw_text(line: &Line) -> String {
    match line {
        Line::Move(Move { raw, .. }) => raw.clone(),
        Line::Comment(c) => c.raw.clone(),
        Line::LayerChange(_) => {
            // Synthesized from a comment marker; the comment line
            // itself is the prior `Line::Comment` entry. Surface a
            // descriptive label so the hover popup doesn't show an
            // empty string.
            ";LAYER_CHANGE".into()
        }
        Line::ToolChange(t) => t.raw.clone(),
        Line::Other(o) => o.raw.clone(),
    }
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
    use std::io::Write;

    fn write_temp_gcode(body: &str) -> PathBuf {
        let mut t = tempfile::NamedTempFile::new().expect("tempfile");
        t.write_all(body.as_bytes()).unwrap();
        t.into_temp_path().keep().unwrap()
    }

    fn fixture_gcode() -> String {
        ";LAYER_CHANGE\n\
         ;Z:0.2\n\
         G1 X0 Y0 Z0.2 F1800\n\
         ;TYPE:External perimeter\n\
         G1 X10 Y0 E0.5 F1200\n\
         G1 X10 Y10 E1.0 F1200\n\
         ;LAYER_CHANGE\n\
         ;Z:0.4\n\
         G1 X0 Y10 E1.5 F1200\n"
            .into()
    }

    #[test]
    fn load_emits_handle_with_correct_counts() {
        let path = write_temp_gcode(&fixture_gcode());
        let reg = Arc::new(PreviewRegistry::new());

        // Drive the command body directly — Tauri's State wrapper
        // is hard to construct in unit tests, so we replicate the
        // body inline (just the pure-Rust slice, no Tauri runtime).
        let src = std::fs::read_to_string(&path).unwrap();
        let lines = parse_str(&src);
        let geom = build_preview(&lines);
        let ls = compute_layer_stats(&geom);
        let js = compute_job_stats(&geom, &ls);
        let handle = reg.alloc_id();
        reg.insert(
            handle,
            LoadedPreview {
                source_path: path.clone(),
                header: HeaderMetadata::default(),
                geometry: geom,
                layer_stats: ls.clone(),
                job_stats: js.clone(),
                lines: lines.clone(),
            },
        );

        // 3 extrusions across 2 layers.
        let segs = reg.with(handle, |p| p.geometry.extrusions.len()).unwrap();
        assert_eq!(segs, 3);
        assert_eq!(js.layer_count, 2);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn segment_detail_surfaces_source_line_text() {
        let src = fixture_gcode();
        let lines = parse_str(&src);
        let geom = build_preview(&lines);
        let ls = compute_layer_stats(&geom);
        let js = compute_job_stats(&geom, &ls);
        let reg = Arc::new(PreviewRegistry::new());
        let handle = reg.alloc_id();
        reg.insert(
            handle,
            LoadedPreview {
                source_path: PathBuf::from("(in-memory)"),
                header: HeaderMetadata::default(),
                geometry: geom,
                layer_stats: ls,
                job_stats: js,
                lines,
            },
        );

        let detail = reg
            .with(handle, |p| -> Result<SegmentDetail, String> {
                let i = 0; // first extrusion segment
                let segs = &p.geometry.extrusions;
                let source_line = segs.source_line[i] as usize;
                Ok(SegmentDetail {
                    source_line_text: line_raw_text(&p.lines[source_line]),
                    start: [segs.positions[0], segs.positions[1], segs.positions[2]],
                    end: [segs.positions[3], segs.positions[4], segs.positions[5]],
                    speed: segs.speed[i],
                    feature: segs.feature[i].clone(),
                    layer_index: segs.layer_index[0] as u32,
                    tool: segs.tool[i],
                    extrusion_mm: 0.5,
                    width: segs.width[i],
                    height: segs.height[i],
                })
            })
            .unwrap()
            .unwrap();

        // First extrusion is `G1 X10 Y0 E0.5 F1200` from the
        // fixture.
        assert!(detail.source_line_text.contains("G1"));
        assert!(detail.source_line_text.contains("X10"));
        assert_eq!(detail.feature, FeatureType::ExternalPerimeter);
    }

    #[test]
    fn gcode_3mf_round_trip_loads_first_plate_with_metadata() {
        use crate::core::slice::PlateSummary;
        use crate::core::threemf::{write_sliced_3mf, AmsBinding, SlicedPlate, SlicedProjectInput};

        // Build a tiny synthetic .gcode.3mf with two plates so we
        // also exercise the multi-plate path.
        let summary = PlateSummary {
            layer_count: 2,
            estimated_time_seconds: 60,
            estimated_time_text: "1m".into(),
            ..Default::default()
        };
        let plate1 = SlicedPlate {
            plate_id: 1,
            gcode: fixture_gcode().into_bytes(),
            summary: summary.clone(),
            thumbnail_png: Some(vec![0xDE, 0xAD]),
            ams_bindings: vec![AmsBinding {
                model_material_index: 0,
                ams_slot: 3,
            }],
        };
        let plate2 = SlicedPlate {
            plate_id: 2,
            gcode: b";LAYER_CHANGE\n;Z:0.2\nG1 X0 Y0 Z0.2 F1800\n".to_vec(),
            summary,
            thumbnail_png: None,
            ams_bindings: vec![],
        };
        let input = SlicedProjectInput {
            printer_model: "Bambu Lab A1 mini".into(),
            file_metadata: std::collections::BTreeMap::new(),
            plates: vec![plate1, plate2],
        };
        let path = std::env::temp_dir().join(format!(
            "n3o-test-3mf-load-{}.gcode.3mf",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        write_sliced_3mf(&input, &path).expect("write");

        // Drive the new command's body (inline; State is hard to
        // synthesize in unit tests).
        let read = crate::core::threemf::read_sliced_3mf(&path).expect("read");
        assert_eq!(read.plates.len(), 2);
        let plate = read.plates.into_iter().next().unwrap();
        let src = String::from_utf8(plate.gcode).expect("utf-8");
        let reg = PreviewRegistry::new();
        let preview = register_preview(&reg, path.clone(), &src);
        assert_eq!(preview.layer_count, 2);
        let meta = plate.metadata.expect("metadata");
        assert_eq!(meta.estimated_time_text, "1m");
        assert_eq!(meta.ams_bindings.len(), 1);
        assert_eq!(meta.ams_bindings[0].ams_slot, 3);
        assert_eq!(
            plate.thumbnail_png.as_deref(),
            Some([0xDE, 0xAD].as_slice())
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn drop_frees_handle() {
        let reg = Arc::new(PreviewRegistry::new());
        let handle = reg.alloc_id();
        reg.insert(
            handle,
            LoadedPreview {
                source_path: PathBuf::from("(in-memory)"),
                header: HeaderMetadata::default(),
                geometry: super::super::ir::PreviewGeometry::default(),
                layer_stats: vec![],
                job_stats: compute_job_stats(&super::super::ir::PreviewGeometry::default(), &[]),
                lines: vec![],
            },
        );
        assert!(reg.remove(handle));
        assert!(reg.with(handle, |_| ()).is_none());
    }
}
