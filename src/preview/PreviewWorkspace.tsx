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

import { useEffect, useState } from "react";

import { ColorModePicker, useColorModePicker } from "./ColorModePicker";
import { GcodePreview } from "./GcodePreview";
import { HoverTooltip } from "./HoverTooltip";
import { LayerSlider } from "./LayerSlider";
import { defaultWindow } from "./layerWindow";
import { previewLayerStats } from "./invokes";
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
}

export function PreviewWorkspace({ preview, bedExtents }: PreviewWorkspaceProps) {
  const { state: colorState, onChange: setColorState } = useColorModePicker();
  const { value: visState, onChange: setVisState } = useVisibilityToggles();

  // Layer window resets to "show all" on each new preview load.
  const [layerWindow, setLayerWindow] = useState<LayerWindow>(() =>
    defaultWindow(preview?.layer_count ?? 0),
  );
  useEffect(() => {
    setLayerWindow(defaultWindow(preview?.layer_count ?? 0));
  }, [preview?.handle]);

  // Per-layer stats fetched once per preview load.
  const [layerStats, setLayerStats] = useState<PerLayerStats[]>([]);
  useEffect(() => {
    if (!preview) {
      setLayerStats([]);
      return;
    }
    let cancelled = false;
    void previewLayerStats(preview.handle)
      .then((stats) => {
        if (!cancelled) setLayerStats(stats);
      })
      .catch((err) =>
        console.error("[preview] previewLayerStats failed", err),
      );
    return () => {
      cancelled = true;
    };
  }, [preview?.handle]);

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
      <div className="preview-workspace">
        <div className="preview-canvas-region">
          <div className="preview-toolbar">
            <ColorModePicker
              mode={colorState.mode}
              palette={colorState.palette}
              onChange={setColorState}
            />
            <VisibilityToggles value={visState} onChange={setVisState} />
          </div>
          <div className="preview-canvas-host">
            <GcodePreview
              preview={preview}
              bedExtents={bedExtents}
              colorMode={colorState.mode}
              palette={colorState.palette}
              layerWindow={layerWindow}
              showTravels={visState.showTravels}
              showRetractions={visState.showRetractions}
              onSegmentHover={setHoverDetail}
            />
          </div>
          <div className="preview-slider-region">
            <LayerSlider
              layerCount={preview?.layer_count ?? 0}
              value={layerWindow}
              onChange={setLayerWindow}
            />
          </div>
        </div>
        {preview && (
          <aside className="preview-stats-column">
            <FullJobStatsPanel
              stats={preview.job_stats}
              header={preview.header}
            />
            <PerLayerStatsPanel
              stats={currentLayerStats}
              layerCount={preview.layer_count}
              rangeMode={layerWindow.mode === "range"}
            />
          </aside>
        )}
      </div>
      <HoverTooltip
        detail={hoverDetail}
        mouseX={hoverPos.x}
        mouseY={hoverPos.y}
        viewportWidth={window.innerWidth}
        viewportHeight={window.innerHeight}
      />
    </>
  );
}
