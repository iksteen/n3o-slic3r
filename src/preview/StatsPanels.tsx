// Per-layer + full-job stats panels (PR-6-12).
//
// Two cards stacked in the right-side panel column when the
// preview mode is active. Both pure presentational over the
// stats returned by PR-6-6 / PR-6-7.
//
// Feature-time bar uses CSS flex with width % rather than a
// chart library — the breakdown is small (≤ 10 features) and
// avoids the dependency.

import type { FullJobStats, HeaderMetadata, PerLayerStats } from "./types";

export interface FullJobStatsPanelProps {
  stats: FullJobStats;
  header: HeaderMetadata;
}

export function FullJobStatsPanel({
  stats,
  header,
}: FullJobStatsPanelProps) {
  const headerTime = header.estimated_time;
  const computedTime = formatDuration(stats.total_duration_seconds);
  const time = headerTime ?? computedTime;

  return (
    <div className="stats-panel job-stats-panel">
      <h3 className="stats-panel-title">Full job</h3>
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
          {formatDuration(stats.duration_seconds)}
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
          <div className="feature-bar" key={feat} title={`${feat}: ${formatDuration(secs)} (${pct.toFixed(1)}%)`}>
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

function formatDuration(seconds: number): string {
  if (seconds <= 0) return "—";
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = Math.floor(seconds % 60);
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${secs}s`;
  return `${secs}s`;
}
