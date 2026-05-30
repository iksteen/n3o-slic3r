// Send-gating predicates shared across send surfaces.
//
// These define the single source of truth for "is the printer in a
// state where we can hand it a new job, and if not, why?". Extracted
// from the topbar SendControls (and originally the now-removed
// PrinterPanel) so the rule lives in one named place — when a new
// JobState is added, the idle/sendable set is updated here, not
// re-derived per component.

import type { JobProgress, PrinterStatus } from "./types";

/** A job state we can safely send over: no job, or one that has
 *  reached a terminal/idle state. Anything mid-run (Printing,
 *  Paused, …) is NOT idle. */
export function isJobIdle(job: JobProgress | null): boolean {
  if (job == null) return true;
  return (
    job.state.state === "Idle" ||
    job.state.state === "Finished" ||
    job.state.state === "Failed"
  );
}

/** Why a send is disabled, for a button tooltip — or "" when it can
 *  proceed. Ordered most- to least-fundamental blocker. */
export function sendDisabledReason(
  printerIdentity: string | null,
  plateId: number | null,
  status: PrinterStatus | null,
  lastSliceOutputPath: string | null,
): string {
  if (printerIdentity == null || plateId == null) return "Bind a printer to send";
  if (status == null) return "Printer not connected";
  if (status.connection.state !== "Connected") {
    return `Printer is ${status.connection.state}`;
  }
  if (lastSliceOutputPath == null) return "Slice the plate first";
  if (!isJobIdle(status.job)) return "Printer is busy";
  return "";
}
