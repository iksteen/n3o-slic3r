// Topbar send controls — Send + Export for the active plate's bound
// printer.
//
// Extracted from the former PrinterPanel: the live monitoring
// (status pill, temps, job line, AMS/toolhead strips, pause/stop) now
// lives in the Devices view, leaving just the actionable send/export
// controls here in the topbar. A richer "Send to printer" modal
// (plate + printer picker) is a deferred follow-up; these direct
// controls send the active plate's last slice to its bound printer.

import { useEffect, useState } from "react";
import { saveFile } from "../ui/fileDialog";
import { driverErrorMessage, driverExportPlate, driverSendPlate } from "./invokes";
import { captureThumbnail } from "../viewport/thumbnailCapture";
import { pushLog } from "../logging/logStore";
import { useDriverStatus } from "./useDriverStatus";
import { beginUpload, endUpload } from "./useUploadProgress";
import { isJobIdle, sendDisabledReason } from "./sendGate";
import type { ConnectionSummary } from "./useDriverConnections";
import type { DriverId, PrinterStatus } from "./types";

export interface SendControlsProps {
  /** Cascade-side printer identity from the active plate's binding,
   *  or `null` if the plate isn't bound yet. */
  printerIdentity: string | null;
  /** Auto-connection summary for the active plate's bound printer —
   *  supplies the live driver id and gates Send on a real connection. */
  connection: ConnectionSummary | null;
  /** Active plate id — needed for the send call's subtask name. */
  plateId: number | null;
  /** Project file name (e.g. `MyPrint.3mf`, or `Untitled.3mf` unsaved) —
   *  contributes the project half of the export default filename. */
  projectName: string;
  /** Active plate's display name (e.g. `Plate 1`, or a renamed `Lid`) —
   *  the plate half of the export default filename. */
  plateName: string | null;
  /** Path of the active plate's most recent slice output, or `null`
   *  until the first slice completes. */
  lastSliceOutputPath: string | null;
  /** Fired after the printer accepts a Send. App uses it to jump to the
   *  Devices monitor for the destination printer (when it's connected) so
   *  the user lands on the live job. SendControls has no instance/connection
   *  context of its own — App supplies it. */
  onSent?: () => void;
}

/** Filename-safe basename, mirroring the backend's `sanitize_basename`:
 *  keep `[A-Za-z0-9._-]`, map the rest to `_`, collapse runs, trim
 *  separators, fall back to `untitled`. Used only for the export
 *  default — the user can still override it in the picker, and the
 *  backend re-derives the printer-visible names on send. */
function sanitizeBasename(s: string): string {
  const collapsed = s
    .replace(/[^A-Za-z0-9._-]/g, "_")
    .replace(/_+/g, "_")
    .replace(/^[._-]+|[._-]+$/g, "");
  return collapsed === "" ? "untitled" : collapsed;
}

/** The "we just sent and the printer hasn't acted yet" latch lives at
 *  module scope, not in component state, because the whole topbar (and
 *  this component) is unmounted while the Devices view is open — a
 *  component-local latch would be lost on a tab round-trip and let a
 *  duplicate send through. There's only ever one SendControls (bound to
 *  the active plate's printer), so a single record suffices. `sinceJob`
 *  is the job-state token at send time, used to release on the first
 *  real change. */
let pendingSend: { driverId: DriverId; sinceJob: string } | null = null;

/** Job-state token for the latch ("Idle" / "Printing" / … / "idle"
 *  when there's no job yet). */
function jobToken(status: PrinterStatus | null): string {
  return status?.job?.state.state ?? "idle";
}

export function SendControls({
  printerIdentity,
  connection,
  plateId,
  projectName,
  plateName,
  lastSliceOutputPath,
  onSent,
}: SendControlsProps): React.JSX.Element | null {
  const driverId = connection?.driverId ?? null;
  const [actionPending, setActionPending] = useState(false);
  // Mirror the module-scoped latch into state (lazy-init from it so a
  // remount after a Devices round-trip restores it). The latch is
  // scoped to the driver we sent to, so switching the active plate to a
  // different printer never inherits a stale "awaiting pickup".
  const [awaitingDriver, setAwaitingDriver] = useState<DriverId | null>(
    () => pendingSend?.driverId ?? null,
  );
  const { status } = useDriverStatus(driverId);
  const awaitingPickup = awaitingDriver != null && awaitingDriver === driverId;

  const setAwaiting = (
    next: { driverId: DriverId; sinceJob: string } | null,
  ): void => {
    pendingSend = next;
    setAwaitingDriver(next?.driverId ?? null);
  };

  // Release the latch on the first real change for the driver we sent
  // to: the job state moved off what it was at send time (picked up,
  // finished, failed…), or the link dropped. Also drop it if the bound
  // driver changed out from under us. Releasing on a transition — not a
  // wall-clock timer — keeps a printer slow to start from re-enabling
  // Send (and re-allowing a duplicate) before it has acted, while still
  // clearing promptly the moment the job state actually changes.
  useEffect(() => {
    if (awaitingDriver == null) return;
    if (awaitingDriver !== driverId) {
      setAwaiting(null);
      return;
    }
    if (status == null) return;
    const changed = jobToken(status) !== (pendingSend?.sinceJob ?? null);
    if (changed || status.connection.state !== "Connected") {
      setAwaiting(null);
    }
  }, [awaitingDriver, driverId, status]);
  // Backstop: if the printer stays connected and its job state never
  // changes (job silently dropped), don't wedge Send forever.
  useEffect(() => {
    if (awaitingDriver == null) return;
    const t = setTimeout(() => setAwaiting(null), 60000);
    return () => clearTimeout(t);
  }, [awaitingDriver]);

  // Send and Export both operate on a sliced bundle — with nothing
  // sliced for this plate yet, there's nothing to act on, so the
  // controls don't appear at all (rather than showing as disabled).
  // After this guard, `lastSliceOutputPath` is narrowed non-null.
  if (lastSliceOutputPath == null) return null;

  const handleSend = async (): Promise<void> => {
    if (driverId == null || plateId == null) return;
    setActionPending(true);
    // Drives the floating SendProgressWindow (over the canvas); cleared in the
    // finally so a failed upload doesn't leave the window stuck.
    beginUpload(driverId);
    try {
      // Render the plate preview off the live viewport; null (empty plate or
      // no viewport) just sends without a thumbnail.
      const thumbnail = captureThumbnail();
      await driverSendPlate(driverId, plateId, lastSliceOutputPath, thumbnail);
      // Accepted — latch Send off (for this driver) until the job state
      // changes from what it is now or the link drops.
      setAwaiting({ driverId, sinceJob: jobToken(status) });
      // Jump the user to the destination printer's live monitor (App decides
      // whether it's connected). Done after the latch so a throw above skips it.
      onSent?.();
    } catch (e) {
      // driver_send_plate rejects with a serialized DriverError: the unit
      // variant Cancelled arrives as the string "Cancelled"; the rest as
      // { Variant: "message" }. A user-initiated cancel isn't a failure —
      // log it quietly and don't latch / jump.
      if (e === "Cancelled") {
        pushLog("info", "Send cancelled");
      } else {
        pushLog("error", `Send failed: ${driverErrorMessage(e)}`);
      }
    } finally {
      setActionPending(false);
      endUpload(driverId);
    }
  };

  const handleExport = async (): Promise<void> => {
    if (plateId == null) return;
    setActionPending(true);
    try {
      // Mirror the backend's combined basename: project title (the file
      // name minus its `.n3o`) + the plate's name. User can override.
      const projectTitle = projectName.replace(/\.n3o$/i, "");
      const plateTitle = plateName ?? `Plate ${plateId}`;
      const defaultName = `${sanitizeBasename(projectTitle)}_${sanitizeBasename(plateTitle)}.gcode.3mf`;
      const path = await saveFile({
        title: "Export .gcode.3mf",
        defaultPath: defaultName,
        filters: [{ name: "Bambu sliced bundle", extensions: ["gcode.3mf"] }],
      });
      if (path == null) {
        // User cancelled the picker.
        return;
      }
      // Embed the same preview the send path would, so the exported bundle
      // is representative (and lets you eyeball Metadata/plate_N.png offline).
      const thumbnail = captureThumbnail();
      await driverExportPlate(plateId, lastSliceOutputPath, path, thumbnail);
    } catch (e) {
      pushLog("error", `Export failed: ${String(e)}`);
    } finally {
      setActionPending(false);
    }
  };

  const sendEnabled =
    status != null &&
    status.connection.state === "Connected" &&
    isJobIdle(status.job) &&
    !actionPending &&
    !awaitingPickup;
  // Export doesn't touch the printer — just wraps + writes to disk,
  // so it's available whenever there's a slice to wrap.
  const exportEnabled = !actionPending;

  return (
    <div className="flex items-center gap-2 text-xs">
      <button
        type="button"
        onClick={() => void handleSend()}
        disabled={!sendEnabled}
        className="tb-btn primary"
        title={
          sendEnabled
            ? "Send to printer"
            : awaitingPickup
              ? "Waiting for the printer to start the last job…"
              : sendDisabledReason(printerIdentity, plateId, status, lastSliceOutputPath)
        }
      >
        Send
      </button>
      <button
        type="button"
        onClick={() => void handleExport()}
        disabled={!exportEnabled}
        className="tb-btn"
        title="Save the .gcode.3mf bundle we'd send to disk (diagnostic)"
      >
        Export
      </button>
    </div>
  );
}
