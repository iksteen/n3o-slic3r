// PR-7a-7 — printer state panel + send button.
//
// Sits in the topbar between Slice and the version-info text.
// Shows live status for the active plate's bound printer; lets
// the user connect, send the most recent slice, run a motion-only
// dry-run, and pause / resume / stop in-flight prints.
//
// The credential dialog is gated behind an explicit "Connect"
// button rather than auto-opening on plate binding — auto-
// registration is a deferred follow-up (the binding panel and
// driver-setup flow are tracked as separate UX concerns).

import { useEffect, useState } from "react";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { BambuAmsStrip } from "./BambuAmsStrip";
import { U1ToolheadStrip } from "./U1ToolheadStrip";
import {
  clearCredentials,
  clearDriverId,
  getBambuCredentials,
  getDriverId,
  getU1Credentials,
  setDriverId,
} from "./credentialsCache";
import {
  driverCommand,
  driverConnect,
  driverDisconnect,
  driverDrySendPlate,
  driverExportPlate,
  driverRegister,
  driverSendPlate,
  driverUnregister,
} from "./invokes";
import { PrinterCredentialsDialog } from "./PrinterCredentialsDialog";
import { useDriverStatus } from "./useDriverStatus";
import type {
  ConnectionState,
  DriverId,
  DriverKind,
  JobProgress,
  PrinterStatus,
  Temps,
} from "./types";

export interface PrinterPanelProps {
  /** Cascade-side printer identity from the active plate's
   * binding (or `null` if the plate isn't bound yet). */
  printerIdentity: string | null;
  /** Which driver kind to register for this printer instance.
   * Derived in App.tsx from the active plate's printer-instance
   * brand (PR-7b-7). `null` when no plate is bound or the
   * derivation hasn't resolved yet (treated as Bambu for the
   * legacy single-printer flow — TODO drop the default when the
   * derivation always resolves). */
  driverKind?: DriverKind | null;
  /** Active plate id — needed for the send call's
   * `subtask_name`. `null` collapses the panel to the bind hint. */
  plateId: number | null;
  /** Path on disk of the most recent slice's `.gcode` output for
   * the active plate. `null` until the first slice completes;
   * Send / Dry-run buttons are disabled when `null`. */
  lastSliceOutputPath: string | null;
}

export function PrinterPanel(props: PrinterPanelProps): React.JSX.Element {
  const { printerIdentity, plateId, lastSliceOutputPath } = props;
  // Default to Bambu when no kind is derived yet — keeps the
  // pre-PR-7b-7 mount path working until App.tsx always passes a
  // resolved kind.
  const driverKind: DriverKind = props.driverKind ?? "Bambu";
  const [driverId, setDriverIdState] = useState<DriverId | null>(null);
  const [showDialog, setShowDialog] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionPending, setActionPending] = useState(false);
  const { status } = useDriverStatus(driverId);

  // Sync `driverId` from the cache whenever the active printer
  // identity changes (e.g. user switches plates). Also attempt a
  // silent register-from-cached-credentials on first mount when
  // we have credentials but no live driver yet.
  useEffect(() => {
    if (printerIdentity == null) {
      setDriverIdState(null);
      return;
    }
    const cachedId = getDriverId(printerIdentity);
    if (cachedId != null) {
      setDriverIdState(cachedId);
      return;
    }
    // Have credentials for the current kind, no live driver →
    // re-register silently. Read the right variant from the cache;
    // mismatched kinds (e.g. plate switched from Bambu to U1)
    // return null and fall through to the connect dialog.
    const bambuCreds = driverKind === "Bambu" ? getBambuCredentials(printerIdentity) : null;
    const u1Creds = driverKind === "U1" ? getU1Credentials(printerIdentity) : null;
    if (bambuCreds == null && u1Creds == null) {
      setDriverIdState(null);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const id = await driverRegister(
          bambuCreds != null
            ? {
                kind: "Bambu",
                data: {
                  host: bambuCreds.host,
                  access_code: bambuCreds.access_code,
                  serial: bambuCreds.serial,
                },
              }
            : {
                kind: "U1",
                data: {
                  host: u1Creds!.host,
                  port: u1Creds!.port,
                  serial: u1Creds!.serial,
                },
              },
        );
        if (cancelled) {
          await driverUnregister(id).catch(() => {});
          return;
        }
        await driverConnect(id);
        if (cancelled) {
          await driverDisconnect(id).catch(() => {});
          await driverUnregister(id).catch(() => {});
          return;
        }
        setDriverId(printerIdentity, id);
        setDriverIdState(id);
      } catch (e) {
        if (!cancelled) {
          setActionError(`Silent reconnect failed: ${String(e)}`);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [printerIdentity]);

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

  const handleDisconnect = async (): Promise<void> => {
    if (driverId == null || printerIdentity == null) return;
    setActionPending(true);
    try {
      await driverDisconnect(driverId).catch(() => {});
      await driverUnregister(driverId).catch(() => {});
    } finally {
      clearDriverId(printerIdentity);
      clearCredentials(printerIdentity);
      setDriverIdState(null);
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
  // Available whenever we have a slice to wrap.
  const exportEnabled = lastSliceOutputPath != null && !actionPending;

  // Export is the only diagnostic that works without a connected
  // driver — just wraps + writes to disk. Always render it (when
  // there's a slice to wrap), so it's reachable from the bind-hint
  // and Connect-button states too.
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

  return (
    <div className="flex items-center gap-2 text-xs">
      {driverId == null ? (
        // No live driver: either no binding (hint) or have binding
        // but not connected yet (Connect button).
        printerIdentity == null || plateId == null ? (
          <span className="text-text-muted">Bind a printer to send</span>
        ) : (
          <>
            <button
              type="button"
              onClick={() => setShowDialog(true)}
              className="px-2 py-1 text-xs border border-border rounded hover:bg-surface-2"
              title={`Connect to ${printerIdentity}`}
            >
              Connect printer
            </button>
            {showDialog && (
              <PrinterCredentialsDialog
                printerIdentity={printerIdentity}
                kind={driverKind}
                initial={
                  (driverKind === "Bambu"
                    ? getBambuCredentials(printerIdentity)
                    : getU1Credentials(printerIdentity)) ?? undefined
                }
                onConnected={(id) => {
                  setDriverId(printerIdentity, id);
                  setDriverIdState(id);
                  setShowDialog(false);
                  setActionError(null);
                }}
                onCancel={() => setShowDialog(false)}
              />
            )}
          </>
        )
      ) : (
        <>
          <ConnectionPill connection={status?.connection ?? null} />
          <JobLine job={status?.job ?? null} />
          <TempsLine temps={status?.temps ?? null} kind={driverKind} />
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
        </>
      )}
      {exportButton}
      {driverId != null && (
        <>
          <CommandButtons
            jobState={status?.job?.state ?? null}
            onCommand={handleCommand}
            disabled={actionPending}
          />
          <button
            type="button"
            onClick={() => void handleDisconnect()}
            className="px-1.5 py-0.5 text-xs text-text-muted hover:text-text"
            title="Disconnect + forget credentials for this session"
            disabled={actionPending}
          >
            Disconnect
          </button>
        </>
      )}
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

function ConnectionPill({
  connection,
}: {
  connection: ConnectionState | null;
}): React.JSX.Element {
  if (connection == null) {
    return <span className="text-text-muted">…</span>;
  }
  const { tone, label } = connectionPillStyle(connection);
  return (
    <span
      className={`px-1.5 py-0.5 rounded text-xs ${tone}`}
      title={connectionPillDetail(connection)}
    >
      {label}
    </span>
  );
}

/** Pure projection for the connection pill — exported for tests. */
export function connectionPillStyle(connection: ConnectionState): {
  tone: string;
  label: string;
} {
  switch (connection.state) {
    case "Connecting":
      return { tone: "bg-warn/20 text-warn", label: "Connecting" };
    case "Connected":
      return { tone: "bg-ok/20 text-ok", label: "Connected" };
    case "Reconnecting":
      return {
        tone: "bg-warn/20 text-warn",
        label: `Reconnecting (${connection.data.in_seconds}s)`,
      };
    case "Disconnected":
      return { tone: "bg-danger/20 text-danger", label: "Disconnected" };
  }
}

function connectionPillDetail(connection: ConnectionState): string {
  if (connection.state === "Disconnected") {
    return `Disconnected: ${connection.data.reason}`;
  }
  if (connection.state === "Reconnecting") {
    return `Retrying in ${connection.data.in_seconds} s`;
  }
  return connection.state;
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
  kind: DriverKind;
}): React.JSX.Element | null {
  if (temps == null) return null;
  // U1 reports 4 independent nozzles; their per-toolhead temps
  // render in `U1ToolheadStrip`. Showing `nozzles[0]` here would
  // double the T0 reading. Bed temp is single-source either way.
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
