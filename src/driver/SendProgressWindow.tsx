// Floating send-upload progress window — the send-path counterpart to
// SlicingWindow. Mounts in App's canvasOverlays (over the canvas, never in the
// topbar) and shows while a send to `driverId` is in flight, sharing the same
// ProgressWindow chrome as slicing.

import { ProgressWindow } from "../ui/ProgressWindow";
import { useUploadProgress } from "./useUploadProgress";
import type { DriverId } from "./types";

export interface SendProgressWindowProps {
  /** Active plate's bound driver, or null. */
  driverId: DriverId | null;
}

export function SendProgressWindow({
  driverId,
}: SendProgressWindowProps): React.JSX.Element | null {
  const { active, progress } = useUploadProgress(driverId);
  if (!active) return null;
  return <ProgressWindow title="Sending…" percent={progress?.percent ?? 0} />;
}
