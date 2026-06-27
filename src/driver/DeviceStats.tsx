// Temperature + fan stats column for the Devices monitor.

import type { PrinterStatus } from "./types";

export interface NozzleSwatch {
  color: string | null;
  label: string;
  /** Tooltip for an empty (colorless) swatch. Defaults to "No filament
   *  loaded"; set to something else when the swatch is empty for a
   *  different reason (e.g. loaded filament not reported while idle). */
  emptyTitle?: string;
}

function TempPill({
  label,
  current,
  target,
  compact,
  swatch,
}: {
  label: string;
  current: number;
  target: number;
  compact?: boolean;
  /** `undefined` → no swatch; otherwise a (possibly empty) swatch. */
  swatch?: NozzleSwatch | null;
}): React.JSX.Element {
  const heating = current < target - 1;
  // Cooling toward ANY target (including a nonzero standby), not just 0.
  const cooling = current > target + 1;
  return (
    <div
      className={`device-temp${compact ? " compact" : ""}${heating ? " heating" : ""}${
        cooling ? " cooling" : ""
      }`}
    >
      <div className="device-temp-label">
        {swatch !== undefined && (
          <span
            className={`device-temp-swatch${swatch?.color ? "" : " empty"}`}
            style={{ background: swatch?.color ?? "transparent" }}
            title={
              swatch?.color
                ? swatch.label
                : (swatch?.emptyTitle ?? "No filament loaded")
            }
          />
        )}
        {label}
      </div>
      <div className="device-temp-value">
        <span className="device-temp-current">{Math.round(current)}°</span>
        <span className="device-temp-arrow">→</span>
        <span className="device-temp-target">{Math.round(target)}°</span>
      </div>
      <div className="device-temp-bar">
        <div
          className="device-temp-bar-fill"
          style={{ width: `${Math.min(100, (current / Math.max(target, 60)) * 100)}%` }}
        />
      </div>
    </div>
  );
}

export function StatsColumn({
  offline,
  status,
  nozzleSwatches,
}: {
  offline: boolean;
  status: PrinterStatus | null;
  nozzleSwatches: (NozzleSwatch | null)[];
}): React.JSX.Element {
  if (offline || status == null) {
    return (
      <div className="device-stats device-stats-offline">
        <div className="dim">No telemetry — printer is offline.</div>
      </div>
    );
  }
  const nozzles = status.temps.nozzles;
  const bed = status.temps.bed;
  const fanSpeed = status.extra.data.fan_speed ?? 0;
  const multi = nozzles.length > 1;
  return (
    <div className="device-stats">
      {multi ? (
        <div className="device-temp-group">
          <div className="device-temp-group-label">Nozzles</div>
          <div className={`device-temp-grid n-${nozzles.length}`}>
            {nozzles.map((nt, i) => (
              <TempPill
                key={i}
                label={`T${i + 1}`}
                current={nt.current}
                target={nt.target}
                compact
                swatch={nozzleSwatches[i] ?? { color: null, label: "" }}
              />
            ))}
          </div>
        </div>
      ) : (
        <TempPill
          label="Nozzle"
          current={nozzles[0]?.current ?? 0}
          target={nozzles[0]?.target ?? 0}
          swatch={nozzleSwatches[0] ?? { color: null, label: "" }}
        />
      )}
      <TempPill label="Bed" current={bed.current} target={bed.target} />
      <div className="device-fan">
        <span className="device-fan-label">Part fan</span>
        <span className="device-fan-value">{fanSpeed}%</span>
        <div className="device-fan-bar">
          <div className="device-fan-bar-fill" style={{ width: `${fanSpeed}%` }} />
        </div>
      </div>
    </div>
  );
}
