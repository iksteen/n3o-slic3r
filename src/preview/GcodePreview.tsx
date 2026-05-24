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
  previewSegmentDetail,
} from "./invokes";
import {
  applyLayerWindow,
  mountPreviewScene,
  pickSegment,
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
  onSegmentHover,
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
  const lastAppliedHandleRef = useRef<PreviewHandle | null>(null);
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene || !preview) return;

    let cancelled = false;
    void fetchBuffers(preview.handle, colorMode, palette)
      .then((bytes) => {
        if (cancelled) return;
        // Decide buffer-rebuild vs color-swap based on whether we
        // *successfully applied* the handle previously, not whether
        // we started a fetch for it (strict-mode double-mount
        // otherwise tracks a cancelled fetch as "applied" and
        // routes the second fetch through the color-swap path,
        // which no-ops because no mesh exists yet).
        const handleChanged = lastAppliedHandleRef.current !== preview.handle;
        if (handleChanged) {
          setPreviewBuffers(scene, bytes, preview);
        } else {
          setPreviewColors(scene, bytes, preview);
        }
        lastAppliedHandleRef.current = preview.handle;
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

  // Hover-inspection raycast (PR-6-11). RAF-throttled so a
  // fast-moving cursor doesn't saturate the invoke channel.
  // Skips entirely when no `onSegmentHover` callback is wired.
  const hoverPendingRef = useRef<{ x: number; y: number } | null>(null);
  const hoverRafRef = useRef<number | null>(null);
  const lastSegmentRef = useRef<number | null>(null);
  useEffect(() => {
    const scene = sceneRef.current;
    const el = containerRef.current;
    if (!scene || !el || !onSegmentHover || !preview) return;

    const onMove = (e: PointerEvent): void => {
      hoverPendingRef.current = { x: e.clientX, y: e.clientY };
      if (hoverRafRef.current != null) return;
      hoverRafRef.current = requestAnimationFrame(() => {
        hoverRafRef.current = null;
        const pending = hoverPendingRef.current;
        if (!pending) return;
        const rect = el.getBoundingClientRect();
        const ndcX = ((pending.x - rect.left) / rect.width) * 2 - 1;
        const ndcY = -((pending.y - rect.top) / rect.height) * 2 + 1;
        const segIdx = pickSegment(scene, ndcX, ndcY);
        if (segIdx == null) {
          if (lastSegmentRef.current != null) {
            lastSegmentRef.current = null;
            onSegmentHover(null);
          }
          return;
        }
        if (segIdx === lastSegmentRef.current) return;
        lastSegmentRef.current = segIdx;
        void previewSegmentDetail(preview.handle, segIdx)
          .then((detail) => {
            // Drop the result if the cursor moved away to a
            // different segment in the meantime.
            if (lastSegmentRef.current === segIdx) {
              onSegmentHover(detail);
            }
          })
          .catch((err) =>
            console.error("[preview] segmentDetail failed", err),
          );
      });
    };
    const onLeave = (): void => {
      if (lastSegmentRef.current != null) {
        lastSegmentRef.current = null;
        onSegmentHover(null);
      }
    };
    el.addEventListener("pointermove", onMove);
    el.addEventListener("pointerleave", onLeave);
    return () => {
      el.removeEventListener("pointermove", onMove);
      el.removeEventListener("pointerleave", onLeave);
      if (hoverRafRef.current != null) {
        cancelAnimationFrame(hoverRafRef.current);
        hoverRafRef.current = null;
      }
    };
  }, [onSegmentHover, preview]);

  return (
    <div
      ref={containerRef}
      className="gcode-preview"
      style={{ width: "100%", height: "100%", position: "relative" }}
    />
  );
}
