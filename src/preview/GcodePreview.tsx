// G-code preview renderer (PR-6-8).
//
// React component that owns the preview's Three.js scene
// lifecycle: mounts on first render, fetches the binary buffer
// via `previewBuffers`, swaps geometry on prop changes
// (`handle`, `colorMode`, `palette`), and disposes the scene
// on unmount.
//
// Pure props — all state (layer window, visibility toggles,
// color mode picker) is owned by sibling components and flowed
// down. The renderer just reflects what the props say.

import { useEffect, useRef } from "react";

import {
  previewBuffers as fetchBuffers,
} from "./invokes";
import {
  applyLayerWindow,
  mountPreviewScene,
  resizePreview,
  setBed,
  setPreviewBuffers,
  setPreviewColors,
  setVisibility,
  type PreviewScene,
} from "./previewScene";
import type {
  BoundingBox,
  ColorMode,
  LayerWindow,
  Palette,
  PreviewHandle,
  PreviewLoadResponse,
  SegmentDetail,
} from "./types";

export interface GcodePreviewProps {
  /** Loaded preview handle + counts. `null` renders an empty
   * scene (just the bed). */
  preview: PreviewLoadResponse | null;
  /** Bed extents for the bed grid. `null` skips the grid (e.g.
   * preview of an external file with no project context). */
  bedExtents: BoundingBox | null;
  colorMode: ColorMode;
  palette: Palette;
  layerWindow: LayerWindow;
  showTravels: boolean;
  showRetractions: boolean;
  /** Hover-inspection callback (PR-6-11 wires the raycast +
   * tooltip). `null` clears the hover state. */
  onSegmentHover?: (detail: SegmentDetail | null) => void;
}

export function GcodePreview({
  preview,
  bedExtents,
  colorMode,
  palette,
  layerWindow,
  showTravels,
  showRetractions,
}: GcodePreviewProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const sceneRef = useRef<PreviewScene | null>(null);

  // Mount + dispose on element lifecycle.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const scene = mountPreviewScene(el);
    sceneRef.current = scene;

    const ro = new ResizeObserver(() => {
      resizePreview(scene, el.clientWidth, el.clientHeight);
    });
    ro.observe(el);

    return () => {
      ro.disconnect();
      scene.dispose();
      sceneRef.current = null;
    };
  }, []);

  // Bed: re-build when extents change.
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene) return;
    setBed(scene, bedExtents);
  }, [bedExtents]);

  // Load buffers when handle / colorMode / palette changes.
  // The "handle changed" path replaces geometry; the
  // "colorMode/palette changed" path swaps only colors.
  const lastHandleRef = useRef<PreviewHandle | null>(null);
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene || !preview) return;

    const handleChanged = lastHandleRef.current !== preview.handle;
    lastHandleRef.current = preview.handle;

    let cancelled = false;
    void fetchBuffers(preview.handle, colorMode, palette)
      .then((bytes) => {
        if (cancelled) return;
        if (handleChanged) {
          setPreviewBuffers(scene, bytes, preview);
        } else {
          setPreviewColors(scene, bytes, preview);
        }
      })
      .catch((err) => {
        console.error("[preview] fetchBuffers failed", err);
      });
    return () => {
      cancelled = true;
    };
  }, [preview, colorMode, palette]);

  // Layer-window updates are just uniform writes — cheap, no
  // need to debounce.
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene) return;
    applyLayerWindow(scene, layerWindow);
  }, [layerWindow]);

  // Visibility toggles flip `material.visible` on the matching
  // meshes — also cheap.
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene) return;
    setVisibility(scene, showTravels, showRetractions);
  }, [showTravels, showRetractions]);

  return (
    <div
      ref={containerRef}
      className="gcode-preview"
      style={{ width: "100%", height: "100%", position: "relative" }}
    />
  );
}
