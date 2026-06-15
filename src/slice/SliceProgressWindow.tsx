// Non-blocking slice-progress window, ported from the design's
// `SlicingWindow` (docs/dev/design/app.jsx + `.slicing-window` in
// styles.css). Floats over the viewport's lower-left while a slice is
// in flight, and owns the Cancel control — the topbar Slice button just
// disables while a slice runs, rather than swapping to a Cancel button.
//
// Divergence from the mockup: the design animates a fixed six-stage
// strip off a fake percent timer. We drive everything from the real
// `useSliceJob` reducer state instead — `percent` and `stage` are the
// libslic3r progress callback's own values (see
// core/slice/orchestrator.rs). libslic3r's stage strings are free-form
// and unordered, so we render the single live stage as the active chip
// rather than fabricating a done/active/upcoming progression we don't
// actually have.

import { ProgressWindow } from "../ui/ProgressWindow";
import type { SliceState } from "./types";

export interface SliceProgressWindowProps {
  state: SliceState;
  /** Objects on the plate being sliced — shown as the head count. */
  objectCount: number;
  /** Cancel the in-flight slice job (wired to `useSliceJob`'s cancel). */
  cancel: () => Promise<void>;
}

/** True while a slice job is occupying the worker (the only states the
 *  window should be visible for). */
function isSliceInFlight(status: SliceState["status"]): boolean {
  return (
    status === "starting" || status === "running" || status === "cancelling"
  );
}

export function SliceProgressWindow({
  state,
  objectCount,
  cancel,
}: SliceProgressWindowProps): React.JSX.Element | null {
  if (!isSliceInFlight(state.status)) return null;

  const pct = Math.max(0, Math.min(100, state.percent));
  const cancelling = state.status === "cancelling";
  // The head label tracks the lifecycle: a cancel in progress reads as
  // "Cancelling", the brief pre-first-event window as "Starting", and
  // the steady state as the design's "Slicing".
  const title = cancelling
    ? "Cancelling"
    : state.status === "starting"
      ? "Starting"
      : "Slicing";
  // libslic3r hasn't named a stage yet during start-up; fall back to a
  // neutral label so the strip never renders empty.
  const stageLabel =
    state.stage.trim().length > 0
      ? state.stage
      : cancelling
        ? "Stopping the slicer…"
        : "Preparing…";

  return (
    <ProgressWindow
      title={title}
      percent={pct}
      count={`${objectCount} object${objectCount !== 1 ? "s" : ""}`}
      footer={<span className="progress-window-stage active">{stageLabel}</span>}
      action={
        <button
          type="button"
          className="progress-window-cancel"
          onClick={() => void cancel()}
          disabled={cancelling}
        >
          {cancelling ? "Cancelling…" : "Cancel"}
        </button>
      }
    />
  );
}
