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
import { SendOptionsDialog } from "./SendOptionsDialog";
import {
  setInstanceSendOptions,
  type PrinterInstance,
  type SendOptions,
} from "../printer/printerInstance";
import { onEvents } from "../state/eventRouter";
import type { ConnectionSummary } from "./useDriverConnections";
import type { DriverId, PrinterStatus, StatusUpdateEvent } from "./types";

export interface SendControlsProps {
  /** Cascade-side printer identity from the active plate's binding,
   *  or `null` if the plate isn't bound yet. */
  printerIdentity: string | null;
  /** The active plate's bound printer instance — supplies the driver
   *  kind (which send options apply) and the sticky per-print options
   *  the send dialog edits. `null` when the plate isn't bound. */
  instance: PrinterInstance | null;
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

/** The "we just sent and the printer hasn't acted yet" latch. The whole
 *  lifecycle — not just the record — lives at module scope, because the
 *  topbar (and this component) is unmounted while the Devices view is
 *  open, and Devices is exactly where prints get cancelled. A
 *  component-scoped release listener misses transitions that complete
 *  while unmounted (idle → printing → cancelled → idle lands back on the
 *  send-time token and the compare never fires), wedging Send until the
 *  backstop. So the release listener + backstop timer are armed at send
 *  time on the app-wide `driver:status_update` stream and run regardless
 *  of what view is mounted; mounted components just mirror the latch. */
let pendingSend: { driverId: DriverId; sinceJob: string } | null = null;
let pendingSendOff: (() => void) | null = null;
let pendingSendBackstop: ReturnType<typeof setTimeout> | null = null;
/** Mounted SendControls instances re-render off this on latch changes. */
const latchWatchers = new Set<() => void>();

/** Job-state token for the latch ("Idle" / "Printing" / … / "idle"
 *  when there's no job yet). */
function jobToken(status: PrinterStatus | null): string {
  return status?.job?.state.state ?? "idle";
}

export function releasePendingSend(): void {
  pendingSend = null;
  pendingSendOff?.();
  pendingSendOff = null;
  if (pendingSendBackstop != null) clearTimeout(pendingSendBackstop);
  pendingSendBackstop = null;
  for (const notify of latchWatchers) notify();
}

/** Latch Send off for `driverId` until its job state moves off what it
 *  was at send time (picked up, finished, cancelled…) or the link
 *  drops. A 60s backstop keeps a silently-dropped job from wedging
 *  Send forever. */
export function armPendingSend(driverId: DriverId, sinceJob: string): void {
  releasePendingSend();
  pendingSend = { driverId, sinceJob };
  pendingSendOff = onEvents<StatusUpdateEvent>(
    ["driver:status_update"],
    (e) => {
      if (pendingSend == null) return;
      if (e.payload.driver_id !== pendingSend.driverId) return;
      const status = e.payload.status;
      if (
        jobToken(status) !== pendingSend.sinceJob ||
        status.connection.state !== "Connected"
      ) {
        releasePendingSend();
      }
    },
  );
  pendingSendBackstop = setTimeout(releasePendingSend, 60000);
  for (const notify of latchWatchers) notify();
}

/** Test seam: the driver id the latch is currently armed for. */
export function pendingSendDriverForTests(): DriverId | null {
  return pendingSend?.driverId ?? null;
}

export function SendControls({
  printerIdentity,
  instance,
  connection,
  plateId,
  projectName,
  plateName,
  lastSliceOutputPath,
  onSent,
}: SendControlsProps): React.JSX.Element | null {
  const driverId = connection?.driverId ?? null;
  const [actionPending, setActionPending] = useState(false);
  const [optionsOpen, setOptionsOpen] = useState(false);
  // Mirror the module-scoped latch into state (lazy-init so a remount
  // after a Devices round-trip restores it). Release logic lives at
  // module scope — this component only re-renders when the latch flips.
  const [awaitingDriver, setAwaitingDriver] = useState<DriverId | null>(
    () => pendingSend?.driverId ?? null,
  );
  const { status } = useDriverStatus(driverId);
  // The latch is scoped to the driver we sent to, so switching the
  // active plate to a different printer never inherits "awaiting".
  const awaitingPickup = awaitingDriver != null && awaitingDriver === driverId;

  useEffect(() => {
    const sync = (): void => setAwaitingDriver(pendingSend?.driverId ?? null);
    latchWatchers.add(sync);
    sync(); // catch a flip between lazy-init and subscribe
    return () => {
      latchWatchers.delete(sync);
    };
  }, []);

  // Send and Export both operate on a sliced bundle — with nothing
  // sliced for this plate yet, there's nothing to act on, so the
  // controls don't appear at all (rather than showing as disabled).
  // After this guard, `lastSliceOutputPath` is narrowed non-null.
  if (lastSliceOutputPath == null) return null;

  // Which send-option protocol the bound printer speaks. Bambu and the
  // U1 both take the four toggles; generic Moonraker has none, so Send
  // skips the dialog there and behaves as before.
  const optionsKind = ((): "bambu" | "u1" | null => {
    const kind = instance?.connection?.kind;
    return kind === "bambu" || kind === "u1" ? kind : null;
  })();

  const handleSendClick = (): void => {
    if (driverId == null || plateId == null) return;
    if (optionsKind != null && instance != null) {
      setOptionsOpen(true);
    } else {
      void doSend(null);
    }
  };

  /** Persist edited options (sticky per instance), then upload. */
  const doSend = async (options: SendOptions | null): Promise<void> => {
    if (driverId == null || plateId == null) return;
    setOptionsOpen(false);
    setActionPending(true);
    // Drives the floating SendProgressWindow (over the canvas); cleared in the
    // finally so a failed upload doesn't leave the window stuck.
    beginUpload(driverId);
    try {
      if (options != null && instance != null) {
        // Persist BEFORE the send — the backend reads the instance's
        // options when it builds the print-start command.
        await setInstanceSendOptions(instance.id, options);
      }
      // Render the plate preview off the live viewport; null (empty plate or
      // no viewport) just sends without a thumbnail.
      const thumbnail = captureThumbnail();
      await driverSendPlate(driverId, plateId, lastSliceOutputPath, thumbnail);
      // Accepted — latch Send off (for this driver) until the job state
      // changes from what it is now or the link drops.
      armPendingSend(driverId, jobToken(status));
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
      {optionsOpen && optionsKind != null && instance != null && (
        <SendOptionsDialog
          kind={optionsKind}
          initial={instance.send_options}
          onSend={(options) => void doSend(options)}
          onCancel={() => setOptionsOpen(false)}
        />
      )}
      <button
        type="button"
        onClick={handleSendClick}
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
