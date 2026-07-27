// Monitor (right pane) for the Devices fleet view — camera, temps,
// current job, filament loadout, and pause/resume/stop controls for the
// selected printer.

import { useEffect, useRef, useState } from "react";
import { useSessionState } from "../ui/useSessionState";
import { useDriverStatus } from "./useDriverStatus";
import { driverCommand } from "./invokes";
import { configForConnection } from "./useDriverConnections";
import type { ConnectionSummary } from "./useDriverConnections";
import { isConnectionUsable } from "../printer/connectionValidation";
import type { PrinterInstance } from "../printer/printerInstance";
import { deriveStatus, printerFree, statusMeta } from "./devicesStatus";
import { CameraPanel } from "./DeviceCamera";
import { CurrentJobPanel } from "./DeviceJob";
import { StatsColumn, type NozzleSwatch } from "./DeviceStats";
import { LoadoutPanel, loadoutFromReport, type LoadoutSlot } from "./DeviceLoadout";
import { FlowDynamics } from "./FlowDynamics";

interface MonitorProps {
  instance: PrinterInstance | null;
  summary: ConnectionSummary | null;
  modelLabel: string;
  onEditPrinter: (id: string) => void;
}

export function DeviceMonitor({
  instance,
  summary,
  modelLabel,
  onEditPrinter,
}: MonitorProps): React.JSX.Element {
  const driverId = summary?.driverId ?? null;
  const { status } = useDriverStatus(driverId);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionPending, setActionPending] = useState(false);
  // Per-printer and session-scoped: each printer remembers its own sub-tab,
  // and the choice survives DevicesView unmounting on top-level tab switches.
  const [tab, setTab] = useSessionState<"status" | "flow">(
    `device.tab.${instance?.id ?? "none"}`,
    "status",
  );
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

      {!notConfigured && (
        <div className="device-monitor-tabs" role="tablist">
          <button
            className={`device-monitor-tab${tab === "status" ? " active" : ""}`}
            role="tab"
            aria-selected={tab === "status"}
            onClick={() => setTab("status")}
            type="button"
          >
            Status
          </button>
          <button
            className={`device-monitor-tab${tab === "flow" ? " active" : ""}`}
            role="tab"
            aria-selected={tab === "flow"}
            onClick={() => setTab("flow")}
            type="button"
          >
            Flow dynamics
          </button>
        </div>
      )}

      {actionError && tab === "status" && (
        <div className="sp-error" role="alert" style={{ margin: "8px 22px 0" }}>
          {actionError}
        </div>
      )}

      {!notConfigured && tab === "status" && (
        <div className="device-monitor-body">
          <div className="device-monitor-left">
            <CameraPanel
              instanceId={instance.id}
              config={
                instance.connection && isConnectionUsable(instance.connection)
                  ? configForConnection(instance.connection)
                  : null
              }
              offline={offline}
            />
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

      {/* Kept mounted across tab switches (visibility toggled, not
          unmounted) so edits + an in-flight calibration survive a trip to
          the Status tab. Keyed by instance id so switching printers
          remounts it fresh — the previous printer's calibration state can't
          bleed through. */}
      {!notConfigured && (
        <div
          className="device-monitor-tabpane"
          style={{ display: tab === "flow" ? "flex" : "none" }}
        >
          <FlowDynamics
            key={instance.id}
            instance={instance}
            driverId={driverId}
            printerFree={printerFree(derived.status)}
          />
        </div>
      )}
    </div>
  );
}
