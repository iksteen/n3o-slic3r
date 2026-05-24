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
import { BambuAmsStrip } from "./BambuAmsStrip";
import {
  clearCredentials,
  clearDriverId,
  getCredentials,
  getDriverId,
  setDriverId,
} from "./credentialsCache";
import {
  driverCommand,
  driverConnect,
  driverDisconnect,
  driverDrySendPlate,
  driverRegister,
  driverSendPlate,
  driverUnregister,
} from "./invokes";
import { PrinterCredentialsDialog } from "./PrinterCredentialsDialog";
import { useDriverStatus } from "./useDriverStatus";
import type {
  ConnectionState,
  DriverId,
  JobProgress,
  PrinterStatus,
  Temps,
} from "./types";

export interface PrinterPanelProps {
  /** Cascade-side printer identity from the active plate's
   * binding (or `null` if the plate isn't bound yet). */
  printerIdentity: string | null;
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
    const creds = getCredentials(printerIdentity);
    if (creds == null) {
      setDriverIdState(null);
      return;
    }
    // Have credentials, no live driver — re-register silently.
    let cancelled = false;
    (async () => {
      try {
        const id = await driverRegister({
          kind: "Bambu",
          data: {
            host: creds.host,
            access_code: creds.access_code,
            serial: creds.serial,
          },
        });
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

  // No active plate / no binding — show the bind-a-printer hint.
  if (printerIdentity == null || plateId == null) {
    return (
      <span className="text-xs text-text-muted">
        Bind a printer to send
      </span>
    );
  }

  // Has binding but no live driver — show Connect entry point.
  if (driverId == null) {
    return (
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
            initial={getCredentials(printerIdentity) ?? undefined}
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
    );
  }

  const handleSend = async (dryRun: boolean): Promise<void> => {
    if (lastSliceOutputPath == null) return;
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

  const handleDisconnect = async (): Promise<void> => {
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

  return (
    <div className="flex items-center gap-2 text-xs">
      <ConnectionPill connection={status?.connection ?? null} />
      <JobLine job={status?.job ?? null} />
      <TempsLine temps={status?.temps ?? null} />
      {status?.extra.kind === "Bambu" && (
        <BambuAmsStrip ams={status.extra.data.ams} />
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

function TempsLine({ temps }: { temps: Temps | null }): React.JSX.Element | null {
  if (temps == null) return null;
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
