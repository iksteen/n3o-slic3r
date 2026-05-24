// Tauri invoke wrappers for the preview commands (PR-6-7).
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

/** Parse the gcode at `path`, build the IR + stats, return a
 * handle the renderer follows up with. */
export function previewLoad(path: string): Promise<PreviewLoadResponse> {
  return invoke<PreviewLoadResponse>("preview_load", { path });
}

/** Drag-drop loader (PR-6-14) for Bambu `.gcode.3mf` containers.
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

/** Fetch the binary buffer (positions + colors + layer indices
 * for extrusions, travels, retractions) for one color mode.
 * Returns the raw bytes; the geometry builder slices them by
 * the counts the load response carried. */
export async function previewBuffers(
  handle: PreviewHandle,
  colorMode: ColorMode,
  palette: Palette,
): Promise<ArrayBuffer> {
  // Tauri's binary Response surfaces as `ArrayBuffer` when
  // invoked. The `invoke<ArrayBuffer>` type tells the IPC layer
  // to skip JSON decoding.
  return invoke<ArrayBuffer>("preview_buffers", {
    handle,
    colorMode,
    palette,
  });
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
