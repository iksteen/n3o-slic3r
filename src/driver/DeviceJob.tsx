// Current-job panel for the Devices monitor.

import type { PrinterStatus } from "./types";
import { formatDuration } from "../ui/formatDuration";

export function CurrentJobPanel({ status }: { status: PrinterStatus | null }): React.JSX.Element {
  const job = status?.job ?? null;
  const printingState =
    job != null && (job.state.state === "Printing" || job.state.state === "Paused");
  if (job == null || !printingState) {
    return (
      <div className="device-job device-job-empty">
        <div className="device-job-empty-title">No job running</div>
        <div className="dim">
          Slice a plate and send it to this printer to start one.
        </div>
      </div>
    );
  }
  const percent = job.percent ?? 0;
  const eta = job.eta_seconds;
  const etaClock =
    eta != null
      ? new Date(Date.now() + eta * 1000).toLocaleTimeString([], {
          hour: "2-digit",
          minute: "2-digit",
        })
      : "—";
  return (
    <div className="device-job">
      <div className="device-job-header">
        <div>
          <div className="device-job-name">{job.file_name ?? "Printing"}</div>
          {job.state.state === "Paused" && (
            <div className="device-job-meta">Paused</div>
          )}
        </div>
        <div className="device-job-percent">{Math.round(percent)}%</div>
      </div>
      <div className="device-job-progress">
        <div className="device-job-progress-fill" style={{ width: `${percent}%` }} />
      </div>
      <div className="device-job-times">
        <div>
          <div className="device-job-time-label">Remaining</div>
          <div className="device-job-time-value">
            {eta != null ? formatDuration(eta) : "—"}
          </div>
        </div>
        <div>
          <div className="device-job-time-label">Layer</div>
          <div className="device-job-time-value">
            {job.current_layer != null && job.total_layers != null
              ? `${job.current_layer} / ${job.total_layers}`
              : "—"}
          </div>
        </div>
        <div>
          <div className="device-job-time-label">ETA</div>
          <div className="device-job-time-value">{etaClock}</div>
        </div>
      </div>
    </div>
  );
}
