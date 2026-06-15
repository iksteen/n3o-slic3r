// Floating send-upload progress window — the send-path counterpart to
// SliceProgressWindow. Mounts in App's canvasOverlays (over the canvas, never
// in the topbar) and shows while a send to `driverId` is in flight, sharing
// the same ProgressWindow chrome as slicing. Owns the Cancel control (the
// topbar Send button just disables while a send runs).

import { ProgressWindow } from "../ui/ProgressWindow";
import { useUploadProgress } from "./useUploadProgress";
import { driverSendCancel } from "./invokes";
import type { DriverId } from "./types";

export interface SendProgressWindowProps {
  /** Active plate's bound driver, or null. */
  driverId: DriverId | null;
}

export function SendProgressWindow({
  driverId,
}: SendProgressWindowProps): React.JSX.Element | null {
  const { active, progress } = useUploadProgress(driverId);
  if (!active || driverId == null) return null;
  return (
    <ProgressWindow
      title="Sending…"
      percent={progress?.percent ?? 0}
      action={
        <button
          type="button"
          className="progress-window-cancel"
          // Fire-and-forget + idempotent: the upload aborts, the send rejects,
          // and SendControls' finally clears this window.
          onClick={() => void driverSendCancel(driverId).catch(() => undefined)}
        >
          Cancel
        </button>
      }
    />
  );
}
