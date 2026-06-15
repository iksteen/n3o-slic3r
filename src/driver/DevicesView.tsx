// DevicesView — fleet monitor (ported from docs/dev/design/DevicesView.jsx).
//
// A top-level mode (alongside Prepare / Preview) that replaces the
// topbar PrinterPanel's monitoring role: a printer rail on the left
// lists every configured instance with a live status dot + print
// progress; selecting one fills the monitor on the right (camera,
// temps, current job, filament loadout, pause/resume/stop).
//
// Data wiring vs the mockup:
//   - status comes from useDriverConnections (per-instance summary) +
//     useDriverStatus (live job/temps/extra) rather than a fake map.
//   - loadout is the instance's synced slot bindings (flattenSlots),
//     resolved to display via the filament catalog — the same data the
//     slot chip strip shows. The driver-reported "currently engaged"
//     slot highlight is not wired yet (cross-driver index mapping).
//   - webcam is stubbed: a disabled-camera icon + "Not implemented".
//   - jump-to-originating-plate is omitted (no job→plate mapping yet).

import { useEffect, useMemo, useRef, useState } from "react";
import { useDriverStatus } from "./useDriverStatus";
import { driverCommand } from "./invokes";
import type { ConnectionSummary } from "./useDriverConnections";
import type { AmsFilament, AmsTray, PrinterStatus } from "./types";
import { cssColorFromHex } from "./colorUtils";
import type { PrinterInstance } from "../printer/printerInstance";
import { usePrinterCatalog } from "../printer/usePrinterCatalog";
import { formatDuration } from "../ui/formatDuration";

// ───────── Status derivation ─────────

type DeviceStatus = "idle" | "preparing" | "printing" | "paused" | "error" | "offline";

interface DerivedStatus {
  status: DeviceStatus;
  /** Short human detail (offline reason / error message), if any. */
  detail: string | null;
  /** 0..100 print progress, when printing/paused. */
  progress: number | null;
  /** True only for the "no connection settings at all" case — the
   *  monitor renders just the header (no telemetry/job/loadout to
   *  show). Set here so the body gate doesn't re-test the raw summary. */
  notConfigured?: boolean;
}

/** Collapse the connection summary + live driver status into the five
 *  monitor states the mockup renders. Not-connected (none / connecting
 *  / failed / reconnecting / disconnected) all read as "offline" with a
 *  reason; a connected driver maps its job state to idle/printing/
 *  paused/error. */
function deriveStatus(
  summary: ConnectionSummary | null,
  status: PrinterStatus | null,
): DerivedStatus {
  if (summary == null || summary.status === "none") {
    return {
      status: "offline",
      detail: "Not configured",
      progress: null,
      notConfigured: true,
    };
  }
  if (summary.status === "failed") {
    return {
      status: "offline",
      detail: summary.reason ?? "Connection failed",
      progress: null,
    };
  }
  if (summary.status === "connecting") {
    return { status: "offline", detail: "Connecting…", progress: null };
  }
  // summary.status === "connected" — transport is up, but we may not
  // have a telemetry frame yet (status null), or the live link may be
  // mid-(re)connect. Don't claim "Idle" until we actually know the job.
  if (status == null) {
    return { status: "offline", detail: "Connecting…", progress: null };
  }
  const cs = status.connection;
  if (cs.state === "Connecting") {
    return { status: "offline", detail: "Connecting…", progress: null };
  }
  if (cs.state === "Disconnected" || cs.state === "Reconnecting") {
    return { status: "offline", detail: cs.data.reason, progress: null };
  }
  const job = status.job;
  const progress = job?.percent ?? null;
  if (job == null) return { status: "idle", detail: null, progress: null };
  switch (job.state.state) {
    case "Preparing":
      return { status: "preparing", detail: "Preparing…", progress: null };
    case "Printing":
      return { status: "printing", detail: null, progress };
    case "Paused":
      return { status: "paused", detail: null, progress };
    case "Failed":
      return { status: "error", detail: job.state.reason, progress: null };
    case "Finished":
    case "Idle":
    default:
      return { status: "idle", detail: null, progress: null };
  }
}

function statusMeta(status: DeviceStatus): { label: string; cls: string } {
  switch (status) {
    case "preparing":
      return { label: "Preparing", cls: "preparing" };
    case "printing":
      return { label: "Printing", cls: "printing" };
    case "paused":
      return { label: "Paused", cls: "paused" };
    case "error":
      return { label: "Error", cls: "error" };
    case "offline":
      return { label: "Offline", cls: "offline" };
    case "idle":
    default:
      return { label: "Idle", cls: "idle" };
  }
}

// ───────── Model label ─────────

function useModelLabels(): (instance: PrinterInstance) => string {
  const catalog = usePrinterCatalog();
  return useMemo(() => {
    const byIdentity = new Map(
      catalog.entries.map((e) => [e.identity, e.profile] as const),
    );
    return (instance: PrinterInstance) => {
      const profile = byIdentity.get(instance.vendor_profile_ref);
      if (!profile) return instance.vendor_profile_ref;
      return `${profile.brand} ${profile.model}`;
    };
  }, [catalog.entries]);
}

// ───────── Printer rail (left) ─────────

interface RailRowProps {
  instance: PrinterInstance;
  summary: ConnectionSummary | null;
  modelLabel: string;
  selected: boolean;
  onSelect: () => void;
}

function RailRow({
  instance,
  summary,
  modelLabel,
  selected,
  onSelect,
}: RailRowProps): React.JSX.Element {
  const { status } = useDriverStatus(summary?.driverId ?? null);
  const derived = deriveStatus(summary, status);
  const meta = statusMeta(derived.status);
  const printing = derived.status === "printing";
  return (
    <button
      className={`device-card${selected ? " active" : ""}`}
      onClick={onSelect}
      type="button"
      title={`${instance.display_name} — ${meta.label}`}
    >
      <div className="device-card-row1">
        <span className={`device-status-dot ${meta.cls}`} />
        <span className="device-card-name">{instance.display_name}</span>
        <span className={`device-card-state ${meta.cls}`}>{meta.label}</span>
      </div>
      <div className="device-card-row2">
        <span className="device-card-model">{modelLabel}</span>
        {printing && derived.progress != null && (
          <span className="device-card-eta">{Math.round(derived.progress)}%</span>
        )}
      </div>
      {printing && derived.progress != null && (
        <div className="device-card-progress">
          <div
            className="device-card-progress-fill"
            style={{ width: `${derived.progress}%` }}
          />
        </div>
      )}
      {derived.status === "error" && derived.detail && (
        <div className="device-card-error">{derived.detail}</div>
      )}
    </button>
  );
}

interface PrinterRailProps {
  instances: PrinterInstance[];
  connections: Record<string, ConnectionSummary>;
  modelLabel: (i: PrinterInstance) => string;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onAddPrinter: () => void;
}

function PrinterRail({
  instances,
  connections,
  modelLabel,
  selectedId,
  onSelect,
  onAddPrinter,
}: PrinterRailProps): React.JSX.Element {
  return (
    <aside className="device-rail">
      <div className="device-rail-header">
        <span>Printers</span>
        <span className="device-rail-count">{instances.length}</span>
      </div>
      <div className="device-rail-list">
        {instances.map((inst) => (
          <RailRow
            key={inst.id}
            instance={inst}
            summary={connections[inst.id] ?? null}
            modelLabel={modelLabel(inst)}
            selected={inst.id === selectedId}
            onSelect={() => onSelect(inst.id)}
          />
        ))}
      </div>
      <button className="device-rail-add" onClick={onAddPrinter} type="button">
        <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
          <path
            d="M6 2v8M2 6h8"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </svg>
        Add printer
      </button>
    </aside>
  );
}

// ───────── Camera (stubbed) ─────────

function CameraPanel(): React.JSX.Element {
  return (
    <div className="device-camera off">
      <div className="device-camera-frame">
        <div className="device-camera-off-msg">
          <svg width="32" height="32" viewBox="0 0 32 32" fill="none" opacity="0.4">
            <rect
              x="3"
              y="8"
              width="20"
              height="16"
              rx="2"
              stroke="currentColor"
              strokeWidth="1.5"
            />
            <path
              d="M23 13l6-3v12l-6-3z"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinejoin="round"
            />
            <path d="M3 3l26 26" stroke="currentColor" strokeWidth="1.5" />
          </svg>
          <div>Webcam</div>
          <div className="dim">Not implemented yet</div>
        </div>
      </div>
    </div>
  );
}

// ───────── Stats column (temps + fan) ─────────

interface NozzleSwatch {
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

function StatsColumn({
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

// ───────── Filament loadout ─────────

interface LoadoutSlot {
  key: string;
  label: string;
  color: string | null;
  name: string | null;
  material: string | null;
  /** The slot currently engaged at the nozzle / held by the grabber,
   *  per the live driver report — same derivation the AMS / toolhead
   *  strips use. */
  active: boolean;
}

/** Filament-name from an AMS/external spool's raw report: `"<type> <brand>"`
 *  (e.g. "PLA Basic"), or just the type, or null when untagged. */
function amsFilamentName(id: AmsFilament): string | null {
  const parts = [id.tray_type, id.sub_brand].filter((p): p is string => !!p);
  return parts.length > 0 ? parts.join(" ") : null;
}

function amsLoadoutRow(
  tray: AmsTray,
  unitId: number,
  multiUnit: boolean,
  activeSlot: number | null,
): LoadoutSlot {
  const id = tray.identity;
  return {
    key: `ams:${unitId}:${tray.id}`,
    label: multiUnit
      ? `${String.fromCharCode(65 + unitId)}:${tray.id + 1}`
      : `${tray.id + 1}`,
    color: id ? cssColorFromHex(id.color) : null,
    name: id ? amsFilamentName(id) : null,
    material: id ? id.tray_type : null,
    active: activeSlot != null && activeSlot === tray.id,
  };
}

/** Project the live driver report into loadout rows — what's *physically*
 *  loaded per the printer's MQTT. Intentionally decoupled from the plate /
 *  slot bindings (those are the slicing assignment, a separate concern the
 *  device panel never reflects). `[]` when offline / nothing reported yet,
 *  so a connection doesn't depend on the user having Synced slots. */
export function loadoutFromReport(status: PrinterStatus | null): LoadoutSlot[] {
  if (status == null) return [];
  const extra = status.extra;
  if (extra.kind === "U1") {
    return extra.data.toolhead_filaments.map((fil, i) => ({
      key: `th:${i}`,
      label: `T${i + 1}`,
      color: fil ? cssColorFromHex(fil.color) : null,
      name: fil ? fil.material_type : null,
      material: fil ? fil.material_type : null,
      active: extra.data.mounted_toolhead === i,
    }));
  }
  const rows: LoadoutSlot[] = [];
  const { ams, external_spool } = extra.data;
  if (ams) {
    const multiUnit = ams.units.length > 1;
    for (const unit of ams.units) {
      for (const tray of unit.trays) {
        rows.push(amsLoadoutRow(tray, unit.id, multiUnit, ams.active_slot));
      }
    }
  }
  // The external spool slot always exists on the A1 mini, so its row
  // stays visible even when nothing is loaded — mirroring how the AMS
  // slots stay visible when empty. The backend populates
  // external_spool only when the external is the engaged tray
  // (tray_now == 254), so its presence == loaded == active.
  rows.push({
    key: "ext",
    label: "Ext",
    color: external_spool ? cssColorFromHex(external_spool.color) : null,
    name: external_spool ? amsFilamentName(external_spool) : null,
    material: external_spool ? external_spool.tray_type : null,
    active: external_spool != null,
  });
  return rows;
}

function LoadoutPanel({
  offline,
  slots,
}: {
  offline: boolean;
  slots: LoadoutSlot[];
}): React.JSX.Element {
  if (offline) {
    return (
      <div className="device-loadout">
        <div className="device-loadout-header">
          <span>Filament loadout</span>
        </div>
        <div className="device-loadout-empty dim">Unknown — printer offline.</div>
      </div>
    );
  }
  const loaded = slots.filter((s) => s.name != null);
  if (slots.length === 0) {
    return (
      <div className="device-loadout">
        <div className="device-loadout-header">
          <span>Filament loadout</span>
        </div>
        <div className="device-loadout-empty dim">No filament reported.</div>
      </div>
    );
  }
  return (
    <div className="device-loadout">
      <div className="device-loadout-header">
        <span>Filament loadout</span>
        <span className="device-loadout-count">
          {loaded.length}/{slots.length}
        </span>
      </div>
      <div className="device-loadout-list">
        {slots.map((s) => (
          <div
            key={s.key}
            className={`device-loadout-row${s.name ? "" : " empty"}${
              s.active && s.name ? " active" : ""
            }`}
            title={
              s.name
                ? `${s.label} · ${s.name}${s.material ? ` (${s.material})` : ""}${
                    s.active ? " — currently engaged" : ""
                  }`
                : `${s.label} — empty`
            }
          >
            <span
              className="device-loadout-swatch"
              style={{ background: s.color ?? "transparent" }}
            />
            <span className="device-loadout-slot">{s.label}</span>
            <span className="device-loadout-name">{s.name ?? "Empty"}</span>
            <span className="device-loadout-mat">{s.material ?? "—"}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ───────── Current job ─────────

function CurrentJobPanel({ status }: { status: PrinterStatus | null }): React.JSX.Element {
  const job = status?.job ?? null;
  const printingState =
    job != null && (job.state.state === "Printing" || job.state.state === "Paused");
  if (job == null || !printingState) {
    return (
      <div className="device-job device-job-empty">
        <div className="device-job-empty-title">No job running</div>
        <div className="dim">
          Slice a plate and send it to this printer to start one.
        </div>
      </div>
    );
  }
  const percent = job.percent ?? 0;
  const eta = job.eta_seconds;
  const etaClock =
    eta != null
      ? new Date(Date.now() + eta * 1000).toLocaleTimeString([], {
          hour: "2-digit",
          minute: "2-digit",
        })
      : "—";
  return (
    <div className="device-job">
      <div className="device-job-header">
        <div>
          <div className="device-job-name">{job.file_name ?? "Printing"}</div>
          {job.state.state === "Paused" && (
            <div className="device-job-meta">Paused</div>
          )}
        </div>
        <div className="device-job-percent">{Math.round(percent)}%</div>
      </div>
      <div className="device-job-progress">
        <div className="device-job-progress-fill" style={{ width: `${percent}%` }} />
      </div>
      <div className="device-job-times">
        <div>
          <div className="device-job-time-label">Remaining</div>
          <div className="device-job-time-value">
            {eta != null ? formatDuration(eta) : "—"}
          </div>
        </div>
        <div>
          <div className="device-job-time-label">Layer</div>
          <div className="device-job-time-value">
            {job.current_layer != null && job.total_layers != null
              ? `${job.current_layer} / ${job.total_layers}`
              : "—"}
          </div>
        </div>
        <div>
          <div className="device-job-time-label">ETA</div>
          <div className="device-job-time-value">{etaClock}</div>
        </div>
      </div>
    </div>
  );
}

// ───────── Monitor (right pane) ─────────

interface MonitorProps {
  instance: PrinterInstance | null;
  summary: ConnectionSummary | null;
  modelLabel: string;
  onEditPrinter: (id: string) => void;
}

function DeviceMonitor({
  instance,
  summary,
  modelLabel,
  onEditPrinter,
}: MonitorProps): React.JSX.Element {
  const driverId = summary?.driverId ?? null;
  const { status } = useDriverStatus(driverId);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionPending, setActionPending] = useState(false);
  // Transient action state belongs to the currently-bound driver.
  // DeviceMonitor stays mounted across printer selection (no key
  // remount), so when the driver changes — switched or deleted — reset
  // it, otherwise a stale "Pause failed" from one printer would show on
  // the next. `driverRef` also lets an in-flight command drop its own
  // result if the driver changed while it was running.
  const driverRef = useRef(driverId);
  useEffect(() => {
    driverRef.current = driverId;
    setActionError(null);
    setActionPending(false);
  }, [driverId]);

  if (instance == null) {
    return (
      <div className="device-monitor device-monitor-empty">
        <div className="dim">Select a printer.</div>
      </div>
    );
  }

  const derived = deriveStatus(summary, status);
  const meta = statusMeta(derived.status);
  const offline = derived.status === "offline";
  const printing = derived.status === "printing";
  const paused = derived.status === "paused";
  // No connection settings at all. There's no telemetry, job, or
  // loadout to show — render just the header (status line + settings
  // cog, so the user can open settings and configure it) and skip the
  // monitor body. Sourced from deriveStatus so the header state and the
  // body gate can't drift apart.
  const notConfigured = derived.notConfigured ?? false;

  const runCommand = async (cmd: "Pause" | "Resume" | "Stop"): Promise<void> => {
    if (driverId == null) return;
    const issuedFor = driverId;
    setActionPending(true);
    setActionError(null);
    try {
      await driverCommand(driverId, cmd);
    } catch (e) {
      // Drop the result if the user switched/deleted the printer while
      // the command was in flight — it belongs to a printer we're no
      // longer showing.
      if (driverRef.current === issuedFor) {
        setActionError(`${cmd} failed: ${String(e)}`);
      }
    } finally {
      if (driverRef.current === issuedFor) {
        setActionPending(false);
      }
    }
  };

  // Loadout = what's *physically* loaded per the live MQTT report — the
  // AMS trays / toolheads the printer reports, NOT the instance's synced
  // slot bindings (that's the slicing assignment, a deliberately separate
  // concern). So filament shows on connect, no Sync required.
  const loadoutSlots: LoadoutSlot[] = loadoutFromReport(offline ? null : status);

  // Per-nozzle filament swatch, also from the live report. Multi-toolhead
  // printers map nozzle i → toolhead i's reported filament; a single-nozzle
  // (AMS) printer's one nozzle is fed by whichever slot is engaged, so it
  // shows the active spool.
  const activeSlot = loadoutSlots.find((s) => s.active) ?? null;
  // Single-nozzle (AMS) printers feed one nozzle from whichever slot is
  // engaged. While printing that's `activeSlot`; while idle the driver
  // doesn't report which tray is threaded, so don't claim "No filament
  // loaded" — infer it when exactly one slot has filament, otherwise
  // show an honest empty-but-unknown swatch.
  const singleNozzleSwatch = (): NozzleSwatch => {
    if (activeSlot) return { color: activeSlot.color, label: activeSlot.name ?? "" };
    const loaded = loadoutSlots.filter((s) => s.name != null || s.color != null);
    if (loaded.length === 1) {
      return { color: loaded[0].color, label: loaded[0].name ?? "" };
    }
    if (loaded.length === 0) return { color: null, label: "" };
    return {
      color: null,
      label: "",
      emptyTitle: "Loaded filament not reported while idle",
    };
  };
  const nozzleSwatches: NozzleSwatch[] =
    instance.extruders.length > 1
      ? instance.extruders.map((_ext, i) => {
          const s = loadoutSlots[i];
          return { color: s?.color ?? null, label: s?.name ?? "" };
        })
      : [singleNozzleSwatch()];

  return (
    <div className="device-monitor">
      <div className="device-monitor-header">
        <div className="device-monitor-title-block">
          <div className="device-monitor-eyebrow">{modelLabel}</div>
          <div className="device-monitor-title">{instance.display_name}</div>
          <div className="device-monitor-sub">
            <span className={`device-status-dot ${meta.cls}`} />
            <span className={`device-monitor-state ${meta.cls}`}>{meta.label}</span>
            {derived.detail && <span className="dim">· {derived.detail}</span>}
          </div>
        </div>

        <div className="device-monitor-controls">
          {printing && (
            <button
              className="device-ctl"
              onClick={() => void runCommand("Pause")}
              disabled={actionPending}
              type="button"
            >
              <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
                <rect x="3" y="2.5" width="2" height="7" fill="currentColor" />
                <rect x="7" y="2.5" width="2" height="7" fill="currentColor" />
              </svg>
              Pause
            </button>
          )}
          {paused && (
            <button
              className="device-ctl primary"
              onClick={() => void runCommand("Resume")}
              disabled={actionPending}
              type="button"
            >
              <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
                <path d="M3 2l7 4-7 4V2z" fill="currentColor" />
              </svg>
              Resume
            </button>
          )}
          {(printing || paused) && (
            <button
              className="device-ctl danger"
              onClick={() => {
                if (window.confirm("Stop the current print? This cannot be undone.")) {
                  void runCommand("Stop");
                }
              }}
              disabled={actionPending}
              type="button"
            >
              <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
                <rect x="2.5" y="2.5" width="7" height="7" fill="currentColor" />
              </svg>
              Stop
            </button>
          )}
          <button
            className="device-ctl ghost"
            title="Printer settings"
            onClick={() => onEditPrinter(instance.id)}
            type="button"
          >
            <svg width="11" height="11" viewBox="0 0 14 14" fill="none">
              <circle cx="7" cy="7" r="2" stroke="currentColor" strokeWidth="1.3" />
              <path
                d="M7 1v2M7 11v2M1 7h2M11 7h2M2.5 2.5l1.5 1.5M10 10l1.5 1.5M2.5 11.5L4 10M10 4l1.5-1.5"
                stroke="currentColor"
                strokeWidth="1.3"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </div>
      </div>

      {actionError && (
        <div className="sp-error" role="alert" style={{ margin: "8px 22px 0" }}>
          {actionError}
        </div>
      )}

      {!notConfigured && (
        <div className="device-monitor-body">
          <div className="device-monitor-left">
            <CameraPanel />
            <CurrentJobPanel status={offline ? null : status} />
          </div>
          <div className="device-monitor-right">
            <StatsColumn
              offline={offline}
              status={status}
              nozzleSwatches={nozzleSwatches}
            />
            <LoadoutPanel offline={offline} slots={loadoutSlots} />
          </div>
        </div>
      )}
    </div>
  );
}

// ───────── Root ─────────

export interface DevicesViewProps {
  instances: PrinterInstance[];
  connections: Record<string, ConnectionSummary>;
  /** Selected printer instance id (controlled). App owns this state so it
   *  survives DevicesView's unmount on every Prepare/Preview tab switch, and
   *  so a Send can pre-select the destination printer before the view mounts.
   *  `null` (or an id no longer in `instances`) falls back to the first
   *  printer. */
  selectedId: string | null;
  /** Commit a rail selection up to App's owned state. */
  onSelectId: (id: string) => void;
  onAddPrinter: () => void;
  onEditPrinter: (id: string) => void;
}

export function DevicesView({
  instances,
  connections,
  selectedId,
  onSelectId,
  onAddPrinter,
  onEditPrinter,
}: DevicesViewProps): React.JSX.Element {
  const modelLabel = useModelLabels();

  // Default to the first printer; keep the selection valid as the
  // instance list changes (e.g. the selected printer is deleted).
  const selected =
    instances.find((i) => i.id === selectedId) ?? instances[0] ?? null;

  return (
    <div className="devices-view">
      <PrinterRail
        instances={instances}
        connections={connections}
        modelLabel={modelLabel}
        selectedId={selected?.id ?? null}
        onSelect={onSelectId}
        onAddPrinter={onAddPrinter}
      />
      <DeviceMonitor
        instance={selected}
        summary={selected ? (connections[selected.id] ?? null) : null}
        modelLabel={selected ? modelLabel(selected) : ""}
        onEditPrinter={onEditPrinter}
      />
    </div>
  );
}
