// Status derivation for the Devices fleet monitor. Pure functions that
// collapse the connection summary + live driver status into the small set
// of monitor states the view renders.

import type { ConnectionSummary } from "./useDriverConnections";
import type { PrinterStatus } from "./types";

export type DeviceStatus =
  | "idle"
  | "preparing"
  | "printing"
  | "paused"
  | "error"
  | "offline";

export interface DerivedStatus {
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
export function deriveStatus(
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

/** True when the machine is stopped and available to take a new job.
 *  "error" counts: a failed or cancelled print is a job outcome, not a
 *  busy machine — the printer sits idle holding the reason, same as
 *  after a finished print. Only an active job (preparing/printing/
 *  paused) or a dead link blocks. */
export function printerFree(status: DeviceStatus): boolean {
  return status === "idle" || status === "error";
}

export function statusMeta(status: DeviceStatus): { label: string; cls: string } {
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
