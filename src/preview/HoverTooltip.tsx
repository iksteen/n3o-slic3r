// Hover-inspection tooltip for the preview (PR-6-11).
//
// Renders next to the cursor when GcodePreview's raycast lands
// on a segment. Edge-flip logic keeps the tooltip inside the
// viewport when the cursor is near the right/bottom edges.

import type { FeatureType, SegmentDetail } from "./types";

export interface HoverTooltipProps {
  detail: SegmentDetail | null;
  /** Pointer position in viewport coordinates (clientX/Y). */
  mouseX: number;
  mouseY: number;
  /** Viewport size for edge-flip math. */
  viewportWidth: number;
  viewportHeight: number;
}

const OFFSET = 12;
const TOOLTIP_WIDTH_GUESS = 240;
const TOOLTIP_HEIGHT_GUESS = 130;

export function HoverTooltip({
  detail,
  mouseX,
  mouseY,
  viewportWidth,
  viewportHeight,
}: HoverTooltipProps) {
  if (!detail) return null;

  // Flip to the cursor's left/top when the tooltip would otherwise
  // clip the viewport edge. Estimates use a rough size constant
  // since we render before measuring — the tooltip is dense + the
  // edges have plenty of margin for the worst-case label.
  const flipX = mouseX + OFFSET + TOOLTIP_WIDTH_GUESS > viewportWidth;
  const flipY = mouseY + OFFSET + TOOLTIP_HEIGHT_GUESS > viewportHeight;
  const left = flipX ? mouseX - OFFSET - TOOLTIP_WIDTH_GUESS : mouseX + OFFSET;
  const top = flipY ? mouseY - OFFSET - TOOLTIP_HEIGHT_GUESS : mouseY + OFFSET;

  const isTravel = featureLabel(detail.feature) === "Travel";

  return (
    <div
      className="preview-hover-tooltip"
      style={{
        position: "fixed",
        left,
        top,
        zIndex: 50,
        pointerEvents: "none",
      }}
      role="tooltip"
    >
      <div className="preview-hover-source">{detail.source_line_text}</div>
      <div className="preview-hover-row">
        <span className="preview-hover-label">Position</span>
        <span className="preview-hover-value">
          ({detail.end[0].toFixed(2)}, {detail.end[1].toFixed(2)},{" "}
          {detail.end[2].toFixed(2)}) mm
        </span>
      </div>
      <div className="preview-hover-row">
        <span className="preview-hover-label">Speed</span>
        <span className="preview-hover-value">
          {(detail.speed * 60).toFixed(0)} mm/min
        </span>
      </div>
      <div className="preview-hover-row">
        <span className="preview-hover-label">Feature</span>
        <span className="preview-hover-value">{featureLabel(detail.feature)}</span>
      </div>
      <div className="preview-hover-row">
        <span className="preview-hover-label">Layer</span>
        <span className="preview-hover-value">{detail.layer_index + 1}</span>
      </div>
      {!isTravel && (
        <div className="preview-hover-row">
          <span className="preview-hover-label">Tool</span>
          <span className="preview-hover-value">T{detail.tool}</span>
        </div>
      )}
      {!isTravel && (
        <div className="preview-hover-row">
          <span className="preview-hover-label">Extrusion</span>
          <span className="preview-hover-value">
            {detail.extrusion_mm.toFixed(3)} mm
          </span>
        </div>
      )}
    </div>
  );
}

/** Human-readable label for a [`FeatureType`]. `Other(string)`
 * surfaces the raw token; unit variants get the canonical
 * Bambu/Orca spelling. */
export function featureLabel(feature: FeatureType): string {
  if (typeof feature === "object" && "Other" in feature) {
    return feature.Other;
  }
  switch (feature) {
    case "Perimeter":
      return "Perimeter";
    case "ExternalPerimeter":
      return "External perimeter";
    case "Infill":
      return "Internal infill";
    case "SolidInfill":
      return "Solid infill";
    case "TopSolidInfill":
      return "Top solid infill";
    case "Bridge":
      return "Bridge infill";
    case "Support":
      return "Support material";
    case "Skirt":
      return "Skirt";
    case "Brim":
      return "Brim";
    case "Travel":
      return "Travel";
  }
}
