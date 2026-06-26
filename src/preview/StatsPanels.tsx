// Per-layer + full-job stats panels.
//
// Two cards stacked in the right-side panel column when the
// preview mode is active. Both pure presentational over the
// stats returned by the preview commands.
//
// Feature-time bar uses CSS flex with width % rather than a
// chart library — the breakdown is small (≤ 10 features) and
// avoids the dependency.

import { useEffect, useMemo } from "react";

import { formatDuration } from "../ui/formatDuration";
import type {
  FullJobStats,
  HeaderMetadata,
  PerLayerStats,
  PreviewLoadGcode3mfResponse,
} from "./types";

export interface FullJobStatsPanelProps {
  stats: FullJobStats;
  header: HeaderMetadata;
  /** Populated when the active preview came from a `.gcode.3mf`
   * drop. Surfaces multi-plate badge, plate-metadata
   * estimated time / AMS bindings, and an inline thumbnail. */
  sliced?: PreviewLoadGcode3mfResponse | null;
}

export function FullJobStatsPanel({
  stats,
  header,
  sliced,
}: FullJobStatsPanelProps) {
  const headerTime = header.estimated_time;
  const computedTime = formatDuration(stats.total_duration_seconds, "—");
  const slicedMeta = sliced?.plate_metadata ?? null;
  const time =
    slicedMeta?.estimated_time_text || headerTime || computedTime;

  // Thumbnail: wrap the byte array in a Blob URL so the <img>
  // tag can render it. URL lifecycle is component-scoped — revoke
  // on unmount / when the thumbnail bytes change to avoid the
  // resource leak that Blob URLs cause by default.
  const thumbBytes = sliced?.thumbnail_png ?? null;
  const thumbUrl = useMemo(() => {
    if (!thumbBytes) return null;
    const blob = new Blob([new Uint8Array(thumbBytes)], { type: "image/png" });
    return URL.createObjectURL(blob);
  }, [thumbBytes]);
  useEffect(() => {
    return () => {
      if (thumbUrl) URL.revokeObjectURL(thumbUrl);
    };
  }, [thumbUrl]);

  return (
    <div className="stats-panel job-stats-panel">
      <h3 className="stats-panel-title">Full job</h3>
      {sliced && sliced.plate_count > 1 && (
        <div
          className="stats-panel-badge multi-plate-badge"
          title={`This .gcode.3mf contains ${sliced.plate_count} plates — the preview only shows plate 1.`}
        >
          Plate 1 of {sliced.plate_count}
        </div>
      )}
      {thumbUrl && (
        <img
          className="stats-panel-thumb"
          src={thumbUrl}
          alt="Plate thumbnail from .gcode.3mf"
        />
      )}
      {header.slicer && (
        <div className="stats-panel-meta">
          {header.slicer}
          {header.slicer_version ? ` ${header.slicer_version}` : ""}
        </div>
      )}
      <div className="stats-panel-row">
        <span className="stats-panel-label">Time</span>
        <span className="stats-panel-value">{time}</span>
      </div>
      <FeatureBars breakdown={stats.feature_breakdown} />
      <FilamentRows usedMm={stats.filament_used_mm} />
      <div className="stats-panel-row">
        <span className="stats-panel-label">Layers</span>
        <span className="stats-panel-value">
          {stats.layer_count}
          {stats.layer_heights.variable && (
            <span
              className="stats-panel-badge"
              title={`${stats.layer_heights.min.toFixed(2)}–${stats.layer_heights.max.toFixed(2)} mm`}
            >
              {" "}
              variable
            </span>
          )}
        </span>
      </div>
      <div className="stats-panel-row">
        <span className="stats-panel-label">Bounding box</span>
        <span className="stats-panel-value">
          {(stats.bounding_box.max[0] - stats.bounding_box.min[0]).toFixed(0)} ×{" "}
          {(stats.bounding_box.max[1] - stats.bounding_box.min[1]).toFixed(0)} ×{" "}
          {(stats.bounding_box.max[2] - stats.bounding_box.min[2]).toFixed(0)} mm
        </span>
      </div>
      {slicedMeta && slicedMeta.ams_bindings.length > 0 && (
        <div className="stats-panel-row">
          <span className="stats-panel-label">AMS bindings</span>
          <span className="stats-panel-value">
            {slicedMeta.ams_bindings
              .map(
                (b) =>
                  `m${b.model_material_index + 1}→slot ${b.ams_slot + 1}`,
              )
              .join(", ")}
          </span>
        </div>
      )}
    </div>
  );
}

export interface PerLayerStatsPanelProps {
  stats: PerLayerStats | null;
  layerCount: number;
  /** Disabled state — range-mode windows don't have a single
   * "current layer". Shows a placeholder. */
  rangeMode: boolean;
}

export function PerLayerStatsPanel({
  stats,
  layerCount,
  rangeMode,
}: PerLayerStatsPanelProps) {
  if (rangeMode) {
    return (
      <div className="stats-panel per-layer-panel">
        <h3 className="stats-panel-title">Per layer</h3>
        <p className="stats-panel-empty">
          Switch to <em>single</em> or <em>up-to</em> layer view to
          see per-layer stats.
        </p>
      </div>
    );
  }
  if (!stats) {
    return (
      <div className="stats-panel per-layer-panel">
        <h3 className="stats-panel-title">Per layer</h3>
        <p className="stats-panel-empty">No layer selected.</p>
      </div>
    );
  }
  return (
    <div className="stats-panel per-layer-panel">
      <h3 className="stats-panel-title">
        Layer {stats.layer_index + 1} of {layerCount}
      </h3>
      <div className="stats-panel-row">
        <span className="stats-panel-label">Z</span>
        <span className="stats-panel-value">
          {stats.z.toFixed(2)} mm (h {stats.layer_height.toFixed(2)})
        </span>
      </div>
      <div className="stats-panel-row">
        <span className="stats-panel-label">Time</span>
        <span className="stats-panel-value">
          {formatDuration(stats.duration_seconds, "—")}
        </span>
      </div>
      <div className="stats-panel-row">
        <span className="stats-panel-label">Max speed</span>
        <span className="stats-panel-value">
          {stats.max_speed.toFixed(0)} mm/s
        </span>
      </div>
      <FilamentRows usedMm={stats.filament_used_mm} compact />
    </div>
  );
}

function FeatureBars({
  breakdown,
}: {
  breakdown: Record<string, number>;
}) {
  const entries = Object.entries(breakdown).sort((a, b) => b[1] - a[1]);
  if (entries.length === 0) return null;
  const total = entries.reduce((acc, [, v]) => acc + v, 0);
  if (total <= 0) return null;
  return (
    <div className="feature-bars" role="presentation">
      {entries.map(([feat, secs]) => {
        const pct = (secs / total) * 100;
        // `feat` is already the canonical display name — the
        // Rust side keys feature_breakdown by FeatureType::as_token()
        // (see core/preview/stats.rs::feature_key).
        return (
          <div className="feature-bar" key={feat} title={`${feat}: ${formatDuration(secs, "—")} (${pct.toFixed(1)}%)`}>
            <span className="feature-bar-label">{feat}</span>
            <div className="feature-bar-track">
              <div
                className="feature-bar-fill"
                style={{ width: `${pct.toFixed(1)}%` }}
              />
            </div>
            <span className="feature-bar-pct">{pct.toFixed(0)}%</span>
          </div>
        );
      })}
    </div>
  );
}

function FilamentRows({
  usedMm,
  compact,
}: {
  usedMm: Record<string, number>;
  compact?: boolean;
}) {
  const entries = Object.entries(usedMm).sort(
    ([a], [b]) => Number(a) - Number(b),
  );
  if (entries.length === 0) return null;
  if (entries.length === 1) {
    // Single tool — collapse to one "Filament: …" row.
    const [, mm] = entries[0];
    return (
      <div className="stats-panel-row">
        <span className="stats-panel-label">
          {compact ? "Filament" : "Filament used"}
        </span>
        <span className="stats-panel-value">
          {(mm / 1000).toFixed(2)} m
        </span>
      </div>
    );
  }
  return (
    <>
      {entries.map(([tool, mm]) => (
        <div className="stats-panel-row" key={tool}>
          <span className="stats-panel-label">T{tool}</span>
          <span className="stats-panel-value">
            {(mm / 1000).toFixed(2)} m
          </span>
        </div>
      ))}
    </>
  );
}

