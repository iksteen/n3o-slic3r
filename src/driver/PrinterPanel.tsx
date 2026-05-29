// Printer state panel + send button — topbar slice between Slice and
// the version info.
//
// Connection lifecycle is owned by `useDriverConnections` (mounted
// in App.tsx) — this panel only consumes the `driverId` that hook
// produces for the active printer identity. It has no buttons to
// connect or disconnect; the user edits credentials in the
// per-printer settings modal and the connection follows.
//
// When `driverId` is null + a printer is bound, the panel renders
// a "configure connection" hint pointing at the settings modal.
// When `driverId` is set, the live status pill + job line + send
// buttons render off the `useDriverStatus(driverId)` snapshot.

import { useState } from "react";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { BambuAmsStrip } from "./BambuAmsStrip";
import { U1ToolheadStrip } from "./U1ToolheadStrip";
import {
  driverCommand,
  driverDrySendPlate,
  driverExportPlate,
  driverSendPlate,
} from "./invokes";
import { useDriverStatus } from "./useDriverStatus";
import type { ConnectionSummary } from "./useDriverConnections";
import type {
  DriverExtra,
  JobProgress,
  PrinterStatus,
  Temps,
} from "./types";

export interface PrinterPanelProps {
  /** Cascade-side printer identity from the active plate's
   * binding (or `null` if the plate isn't bound yet). */
  printerIdentity: string | null;
  /** Auto-connection summary for the active plate's bound printer
   *  instance, owned by `useDriverConnections`. `null` when the
   *  active plate is unbound. Drives both the live driver id (for
   *  Send/Pause/Stop) and the empty-state messaging — instead of
   *  inferring "not configured" from a bare null driver id, the
   *  panel branches on `summary.status` so the failed/connecting
   *  states surface distinct copy. */
  connection: ConnectionSummary | null;
  /** Active plate id — needed for the send call's
   * `subtask_name`. `null` collapses the panel to the bind hint. */
  plateId: number | null;
  /** Path on disk of the most recent slice's `.gcode` output for
   * the active plate. `null` until the first slice completes;
   * Send / Dry-run buttons are disabled when `null`. */
  lastSliceOutputPath: string | null;
}

export function PrinterPanel(props: PrinterPanelProps): React.JSX.Element {
  const { printerIdentity, connection, plateId, lastSliceOutputPath } = props;
  const driverId = connection?.driverId ?? null;
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionPending, setActionPending] = useState(false);
  const { status } = useDriverStatus(driverId);

  const handleSend = async (dryRun: boolean): Promise<void> => {
    if (driverId == null || plateId == null || lastSliceOutputPath == null) return;
    setActionPending(true);
    setActionError(null);
    try {
      const send = dryRun ? driverDrySendPlate : driverSendPlate;
      await send(driverId, plateId, lastSliceOutputPath);
    } catch (e) {
      setActionError(`Send failed: ${String(e)}`);
    } finally {
      setActionPending(false);
    }
  };

  const handleCommand = async (cmd: "Pause" | "Resume" | "Stop"): Promise<void> => {
    if (driverId == null) return;
    setActionPending(true);
    setActionError(null);
    try {
      await driverCommand(driverId, cmd);
    } catch (e) {
      setActionError(`${cmd} failed: ${String(e)}`);
    } finally {
      setActionPending(false);
    }
  };

  const handleExport = async (): Promise<void> => {
    if (plateId == null || lastSliceOutputPath == null) return;
    setActionPending(true);
    setActionError(null);
    try {
      const path = await saveDialog({
        title: "Export .gcode.3mf",
        defaultPath: `plate-${plateId}.gcode.3mf`,
        filters: [{ name: "Bambu sliced bundle", extensions: ["gcode.3mf"] }],
      });
      if (path == null) {
        // User cancelled the picker.
        return;
      }
      await driverExportPlate(plateId, lastSliceOutputPath, path);
    } catch (e) {
      setActionError(`Export failed: ${String(e)}`);
    } finally {
      setActionPending(false);
    }
  };

  const sendEnabled =
    status != null &&
    status.connection.state === "Connected" &&
    lastSliceOutputPath != null &&
    isJobIdle(status.job) &&
    !actionPending;

  // Export doesn't touch the printer — just wraps + writes to disk.
  // Available whenever we have a slice to wrap, including the
  // "no driver" hint state below.
  const exportEnabled = lastSliceOutputPath != null && !actionPending;
  const exportButton = (
    <button
      type="button"
      onClick={() => void handleExport()}
      disabled={!exportEnabled}
      className="px-2 py-1 border border-border rounded text-xs hover:bg-surface-2 disabled:opacity-40"
      title="Save the .gcode.3mf bundle we'd send to disk (diagnostic)"
    >
      Export
    </button>
  );

  // Empty / not-configured states. Branch on the reconciler's
  // ConnectionSummary so the picker dot and this hint agree about
  // what's wrong (the dot already distinguishes none / connecting
  // / failed / connected).
  if (printerIdentity == null || plateId == null) {
    return (
      <div className="flex items-center gap-2 text-xs">
        <span className="text-text-muted">Bind a printer to send</span>
        {exportButton}
      </div>
    );
  }
  const summaryStatus = connection?.status ?? "none";
  if (summaryStatus === "none") {
    return (
      <div className="flex items-center gap-2 text-xs">
        <span
          className="text-text-muted"
          title="Open the printer settings (cog in the picker) and fill in the Connection tab"
        >
          Connection not configured
        </span>
        {exportButton}
      </div>
    );
  }
  if (summaryStatus === "connecting") {
    // Distinguish an INITIAL connect (no live driver / no active job
    // yet) from a RECONNECT blip of a driver that's mid-print. On a
    // reconnect the driverId is still live and the last-known job is
    // non-idle — collapsing to a bare "Connecting…" would hide the
    // job progress AND the Pause/Stop controls for the whole backoff
    // window, leaving the user unable to stop a running print. In
    // that case fall through to the live panel (which shows a
    // "Reconnecting…" badge); Send stays disabled because it gates on
    // a live `Connected` status.
    if (driverId == null || isJobIdle(status?.job ?? null)) {
      return (
        <div className="flex items-center gap-2 text-xs">
          <span className="text-text-muted">Connecting…</span>
          {exportButton}
        </div>
      );
    }
  }
  if (summaryStatus === "failed") {
    const reason = connection?.reason ?? null;
    return (
      <div className="flex items-center gap-2 text-xs">
        <span
          className="text-danger truncate max-w-xs"
          title={reason ? `Connection failed: ${reason}` : "Connection failed"}
        >
          Connection failed{reason ? `: ${reason}` : ""}
        </span>
        {exportButton}
      </div>
    );
  }
  // `connected` — fall through to the live status panel below. If
  // driverId is null in this branch (shouldn't happen but be
  // defensive), bail to the connecting state.
  if (driverId == null) {
    return (
      <div className="flex items-center gap-2 text-xs">
        <span className="text-text-muted">Connecting…</span>
        {exportButton}
      </div>
    );
  }

  // Live panel. When we reached here via the reconnect fall-through
  // (status still "connecting", driver mid-job) flag it so the user
  // knows the link is re-establishing while Pause/Stop stay live.
  const reconnecting = summaryStatus !== "connected";
  const reconnectReason = connection?.reason ?? null;
  return (
    <div className="flex items-center gap-2 text-xs">
      {reconnecting && (
        <span
          className="text-amber-500"
          title={`Connection dropped — reconnecting${
            reconnectReason ? ` (${reconnectReason})` : ""
          }. Pause/Stop may report an error until the link is back; live updates resume once reconnected.`}
        >
          Reconnecting…
        </span>
      )}
      <JobLine job={status?.job ?? null} />
      <TempsLine temps={status?.temps ?? null} kind={status?.extra.kind ?? null} />
      {status?.extra.kind === "Bambu" && (
        <BambuAmsStrip ams={status.extra.data.ams} />
      )}
      {status?.extra.kind === "U1" && (
        <U1ToolheadStrip extra={status.extra.data} temps={status.temps} />
      )}
      <button
        type="button"
        onClick={() => void handleSend(false)}
        disabled={!sendEnabled}
        className="px-2 py-1 bg-emerald-700 hover:bg-emerald-600 disabled:opacity-40 rounded text-xs font-medium text-white"
        title={
          sendEnabled
            ? "Send to printer"
            : sendDisabledReason(status, lastSliceOutputPath)
        }
      >
        Send
      </button>
      <button
        type="button"
        onClick={() => void handleSend(true)}
        disabled={!sendEnabled}
        className="px-2 py-1 border border-border rounded text-xs hover:bg-surface-2 disabled:opacity-40"
        title="Dry-run: every motion executes but cold (no heating, no extrusion)"
      >
        Dry-run
      </button>
      {exportButton}
      <CommandButtons
        jobState={status?.job?.state ?? null}
        onCommand={handleCommand}
        disabled={actionPending}
      />
      {actionError && (
        <span
          className="text-xs text-danger truncate max-w-xs"
          role="alert"
          title={actionError}
        >
          {actionError}
        </span>
      )}
    </div>
  );
}

function JobLine({ job }: { job: JobProgress | null }): React.JSX.Element {
  if (job == null || job.state.state === "Idle") {
    return <span className="text-text-muted">Idle</span>;
  }
  return (
    <span className="text-text font-mono">
      {formatJobLine(job)}
    </span>
  );
}

/** Exported for tests. */
export function formatJobLine(job: JobProgress): string {
  const parts: string[] = [];
  if (job.file_name) parts.push(job.file_name);
  if (job.current_layer != null && job.total_layers != null) {
    parts.push(`L ${job.current_layer}/${job.total_layers}`);
  }
  if (job.percent != null) {
    parts.push(`${Math.round(job.percent)}%`);
  }
  if (job.eta_seconds != null) {
    parts.push(`ETA ${formatEta(job.eta_seconds)}`);
  }
  if (job.state.state === "Failed") {
    parts.push(`FAILED: ${job.state.reason}`);
  } else if (job.state.state === "Paused") {
    parts.push("PAUSED");
  }
  return parts.join(" · ");
}

function formatEta(totalSeconds: number): string {
  const h = Math.floor(totalSeconds / 3600);
  const m = Math.floor((totalSeconds % 3600) / 60);
  const s = totalSeconds % 60;
  if (h > 0) {
    return `${h}h${m.toString().padStart(2, "0")}`;
  }
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function TempsLine({
  temps,
  kind,
}: {
  temps: Temps | null;
  /** The live driver's own kind, off `PrinterStatus.extra` — the
   *  authoritative source for which hardware is actually connected,
   *  rather than a profile-derived guess. `null` only when there's
   *  no status yet (in which case `temps` is also null). */
  kind: DriverExtra["kind"] | null;
}): React.JSX.Element | null {
  if (temps == null || kind == null) return null;
  // U1 reports 4 independent nozzles; their per-toolhead temps
  // render in `U1ToolheadStrip`. Showing `nozzles[0]` here would
  // double the T1 reading. Bed temp is single-source either way.
  if (kind === "U1") {
    return (
      <span className="text-text-muted font-mono">
        B {formatTemp(temps.bed)}
      </span>
    );
  }
  const nozzle = temps.nozzles[0];
  return (
    <span className="text-text-muted font-mono">
      N {formatTemp(nozzle)} · B {formatTemp(temps.bed)}
    </span>
  );
}

function formatTemp(reading: { current: number; target: number } | undefined): string {
  if (!reading) return "—";
  return `${Math.round(reading.current)}/${Math.round(reading.target)}°`;
}

function CommandButtons({
  jobState,
  onCommand,
  disabled,
}: {
  jobState: JobProgress["state"] | null;
  onCommand(cmd: "Pause" | "Resume" | "Stop"): void;
  disabled: boolean;
}): React.JSX.Element {
  if (jobState == null) return <></>;
  const showPause = jobState.state === "Printing";
  const showResume = jobState.state === "Paused";
  const showStop = jobState.state === "Printing" || jobState.state === "Paused";
  return (
    <>
      {showPause && (
        <button
          type="button"
          onClick={() => onCommand("Pause")}
          className="px-2 py-1 border border-border rounded text-xs hover:bg-surface-2"
          disabled={disabled}
        >
          Pause
        </button>
      )}
      {showResume && (
        <button
          type="button"
          onClick={() => onCommand("Resume")}
          className="px-2 py-1 border border-border rounded text-xs hover:bg-surface-2"
          disabled={disabled}
        >
          Resume
        </button>
      )}
      {showStop && (
        <button
          type="button"
          onClick={() => {
            if (window.confirm("Stop the current print? This cannot be undone.")) {
              onCommand("Stop");
            }
          }}
          className="px-2 py-1 border border-danger/50 text-danger rounded text-xs hover:bg-danger/10"
          disabled={disabled}
        >
          Stop
        </button>
      )}
    </>
  );
}

function isJobIdle(job: JobProgress | null): boolean {
  if (job == null) return true;
  return (
    job.state.state === "Idle" ||
    job.state.state === "Finished" ||
    job.state.state === "Failed"
  );
}

/** Exported for tests. */
export function sendDisabledReason(
  status: PrinterStatus | null,
  lastSliceOutputPath: string | null,
): string {
  if (status == null) return "Waiting for status…";
  if (status.connection.state !== "Connected") {
    return `Printer is ${status.connection.state}`;
  }
  if (lastSliceOutputPath == null) {
    return "Slice the plate first";
  }
  if (!isJobIdle(status.job)) {
    return "Printer is busy";
  }
  return "";
}
