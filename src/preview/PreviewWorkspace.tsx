// PreviewWorkspace — composes the renderer + slider + toggles +
// hover tooltip + stats panels into one UI region (PR-6-15).
//
// Owns all preview-mode state: color/palette (persisted),
// visibility toggles (persisted), layer window (per-load), the
// most recent hover detail (local), and the cached preview
// handles per plate.
//
// App.tsx mounts this in place of ViewportCanvas when the mode
// toggle flips to "preview". The component takes the active
// plate's preview-load response + bed extents as inputs; the
// App is responsible for the plate-switch + slice-finished
// auto-load (see useSlicePreviewBridge).

import { useEffect, useState, type ReactNode } from "react";

import { ColorModePicker, useColorModePicker } from "./ColorModePicker";
import { DropZone, type DroppedPreview } from "./DropZone";
import { GcodePreview } from "./GcodePreview";
import { HoverTooltip } from "./HoverTooltip";
import { LayerSlider } from "./LayerSlider";
import { defaultWindow } from "./layerWindow";
import { previewDrop, previewLayerStats } from "./invokes";
import { FullJobStatsPanel, PerLayerStatsPanel } from "./StatsPanels";
import { VisibilityToggles, useVisibilityToggles } from "./VisibilityToggles";
import type {
  BoundingBox,
  LayerWindow,
  PerLayerStats,
  PreviewLoadResponse,
  SegmentDetail,
} from "./types";

export interface PreviewWorkspaceProps {
  preview: PreviewLoadResponse | null;
  bedExtents: BoundingBox | null;
  /** Canvas toolbar content (the Prepare/Preview toggle), rendered top-left
   *  inside the canvas frame. */
  toolbar: ReactNode;
  /** Canvas overlays shared with prepare (slicing-progress window + error
   *  console), rendered inside the canvas frame so they anchor to the view. */
  overlays: ReactNode;
}

export function PreviewWorkspace({
  preview,
  bedExtents,
  toolbar,
  overlays,
}: PreviewWorkspaceProps) {
  const { state: colorState, onChange: setColorState } = useColorModePicker();
  const { value: visState, onChange: setVisState } = useVisibilityToggles();

  // Drag-drop loader (PR-6-14). When a user drops a file, the
  // dropped preview overrides the plate's sliced preview until
  // the plate's slice prop changes (re-slice / plate switch).
  const [dropped, setDropped] = useState<DroppedPreview | null>(null);
  const [dropError, setDropError] = useState<string | null>(null);
  useEffect(() => {
    // Plate-switch / re-slice clears the dropped override. Free
    // the handle the drop registered so we don't leak ~250MB per
    // file.
    if (dropped) {
      void previewDrop(dropped.preview.handle).catch(() => undefined);
      setDropped(null);
    }
    // Only refire on slice-side changes — `dropped` in the dep
    // list would re-clear immediately.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [preview?.handle]);

  const activePreview = dropped?.preview ?? preview;

  // Layer window resets to "show all" on each new preview load.
  const [layerWindow, setLayerWindow] = useState<LayerWindow>(() =>
    defaultWindow(activePreview?.layer_count ?? 0),
  );
  useEffect(() => {
    setLayerWindow(defaultWindow(activePreview?.layer_count ?? 0));
  }, [activePreview?.handle]);

  // Per-layer stats fetched once per preview load.
  const [layerStats, setLayerStats] = useState<PerLayerStats[]>([]);
  useEffect(() => {
    if (!activePreview) {
      setLayerStats([]);
      return;
    }
    let cancelled = false;
    void previewLayerStats(activePreview.handle)
      .then((stats) => {
        if (!cancelled) setLayerStats(stats);
      })
      .catch((err) =>
        console.error("[preview] previewLayerStats failed", err),
      );
    return () => {
      cancelled = true;
    };
  }, [activePreview?.handle]);

  // Hover-inspection state.
  const [hoverDetail, setHoverDetail] = useState<SegmentDetail | null>(null);
  const [hoverPos, setHoverPos] = useState<{ x: number; y: number }>({
    x: 0,
    y: 0,
  });
  useEffect(() => {
    const onMove = (e: PointerEvent): void => {
      setHoverPos({ x: e.clientX, y: e.clientY });
    };
    window.addEventListener("pointermove", onMove);
    return () => window.removeEventListener("pointermove", onMove);
  }, []);

  const currentLayer =
    layerWindow.mode === "single"
      ? layerWindow.layer
      : layerWindow.mode === "up-to"
        ? layerWindow.max
        : null;

  const currentLayerStats =
    currentLayer != null
      ? layerStats.find((s) => s.layer_index === currentLayer) ?? null
      : null;

  return (
    <>
      {/* Center column: the canvas frame on top, the layer-slider footer
          below it (outside the frame, so overlays never overlap it). */}
      <div className="canvas-column">
        <div className="canvas-stage">
          <div className="preview-canvas-host">
            <GcodePreview
              preview={activePreview}
              bedExtents={bedExtents}
              colorMode={colorState.mode}
              palette={colorState.palette}
              layerWindow={layerWindow}
              showTravels={visState.showTravels}
              showRetractions={visState.showRetractions}
              onSegmentHover={setHoverDetail}
            />
            <DropZone
              onLoaded={(result) => {
                setDropError(null);
                setDropped(result);
              }}
              onError={(msg) => setDropError(msg)}
            />
            {dropError && (
              <div className="preview-drop-error" role="alert">
                {dropError}
                <button
                  type="button"
                  className="preview-drop-error-dismiss"
                  onClick={() => setDropError(null)}
                  aria-label="Dismiss"
                >
                  ×
                </button>
              </div>
            )}
          </div>
          <div className="canvas-toolbar">{toolbar}</div>
          {overlays}
          <HoverTooltip
            detail={hoverDetail}
            mouseX={hoverPos.x}
            mouseY={hoverPos.y}
            viewportWidth={window.innerWidth}
            viewportHeight={window.innerHeight}
          />
        </div>
        <div className="layer-slider-footer">
          <LayerSlider
            layerCount={activePreview?.layer_count ?? 0}
            value={layerWindow}
            onChange={setLayerWindow}
          />
        </div>
      </div>
      {/* Right column: preview details (controls + stats). Always present so
          the column reserves its width; content appears once a preview loads. */}
      <aside className="preview-details">
        {activePreview && (
          <>
            <div className="preview-controls">
              <ColorModePicker
                mode={colorState.mode}
                palette={colorState.palette}
                onChange={setColorState}
              />
              <VisibilityToggles value={visState} onChange={setVisState} />
            </div>
            <FullJobStatsPanel
              stats={activePreview.job_stats}
              header={activePreview.header}
              sliced={dropped?.sliced ?? null}
            />
            <PerLayerStatsPanel
              stats={currentLayerStats}
              layerCount={activePreview.layer_count}
              rangeMode={layerWindow.mode === "range"}
            />
          </>
        )}
      </aside>
    </>
  );
}
