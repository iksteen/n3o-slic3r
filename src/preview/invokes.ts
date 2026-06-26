// Tauri invoke wrappers for the preview commands.
//
// Thin layer over `invoke()` so the panel + renderer don't need
// to remember command names + arg keys. Binary buffers are
// fetched as `ArrayBuffer` (Tauri's binary Response mode);
// JSON-shaped responses use the typed wrappers.

import { invoke } from "@tauri-apps/api/core";

import type {
  ColorMode,
  Palette,
  PerLayerStats,
  PreviewHandle,
  PreviewLoadGcode3mfResponse,
  PreviewLoadResponse,
  SegmentDetail,
} from "./types";

/** Camera + render parameters for one toolpath frame. The renderer
 * owns the GPU buffers (uploaded once per handle); this is the
 * per-frame state. `layer_min`/`layer_max` are the inclusive window
 * the shader culls against; `color_mode`/`palette` pick the
 * per-instance colors. */
export interface ToolpathFrameReq {
  handle: PreviewHandle;
  width: number;
  height: number;
  az: number;
  el: number;
  dist: number;
  center: [number, number, number];
  layer_min: number;
  layer_max: number;
  color_mode: ColorMode;
  palette: Palette;
  show_travels: boolean;
  show_retractions: boolean;
  /** Bed extents for the floor grid (mm). null skips the grid. */
  bed_min: [number, number, number] | null;
  bed_max: [number, number, number] | null;
}

/** Parse the gcode at `path`, build the IR + stats, return a
 * handle the renderer follows up with. */
export function previewLoad(path: string): Promise<PreviewLoadResponse> {
  return invoke<PreviewLoadResponse>("preview_load", { path });
}

/** Drag-drop loader for Bambu `.gcode.3mf` containers.
 * Unwraps the container, loads plate 1's embedded gcode via the
 * same pipeline as `previewLoad`, and surfaces the pre-baked
 * plate metadata + optional thumbnail for the stats panel. */
export function previewLoadGcode3mf(
  path: string,
): Promise<PreviewLoadGcode3mfResponse> {
  return invoke<PreviewLoadGcode3mfResponse>("preview_load_gcode_3mf", {
    path,
  });
}

/** Render one toolpath frame Rust-side (wgpu, offscreen) and get it
 * back as tight RGBA8 (`ArrayBuffer`) to blit into the canvas. The
 * geometry stays GPU-resident — only pixels cross the bridge. */
export function toolpathFrame(req: ToolpathFrameReq): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("toolpath_frame", { req });
}

/** Cursor → nearest visible extrusion segment index (or null). The
 * pick is a Rust ray-vs-segment sweep over the IR, window-filtered so
 * only on-screen segments hit. */
export function toolpathPick(req: {
  handle: PreviewHandle;
  width: number;
  height: number;
  x: number;
  y: number;
  az: number;
  el: number;
  dist: number;
  center: [number, number, number];
  layer_min: number;
  layer_max: number;
}): Promise<number | null> {
  return invoke<number | null>("toolpath_pick", { req });
}

export function previewLayerStats(
  handle: PreviewHandle,
): Promise<PerLayerStats[]> {
  return invoke<PerLayerStats[]>("preview_layer_stats", { handle });
}

export function previewSegmentDetail(
  handle: PreviewHandle,
  segmentIndex: number,
): Promise<SegmentDetail> {
  return invoke<SegmentDetail>("preview_segment_detail", {
    handle,
    segmentIndex,
  });
}

/** Drop the preview's geometry + line stream (~250MB for a 50MB
 * gcode). Required after switching to a different gcode or
 * closing the preview. */
export function previewDrop(handle: PreviewHandle): Promise<void> {
  return invoke("preview_drop", { handle });
}
