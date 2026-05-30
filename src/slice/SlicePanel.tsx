// Slice / Cancel button for the topbar (PR-3-4, rewired in PR-6-3).
// Post-slice stats (time / filament / layers) and the clear button
// were dropped from the header — the slice result surfaces in Preview.
// In-flight progress moved out of the topbar into the floating
// `SlicingWindow` over the canvas (design's `.slicing-window`); this
// panel is now just the action button + any start/failure error text.
//
// Post-PR-6-3 the Slice button drives off live project state via
// `slice_active_plate` — no file picker, no model-path tracking.
// The backend builds the SliceJobInput from the active plate's
// scene + bindings + overrides and writes its own temp .3mf for
// libslic3r to load. Output gcode lands in a per-job temp dir;
// the path appears on `slice:plate_finished` events.
//
// The `useSliceJob` reducer state is owned by `App` (so the corner
// window can read the same job) and threaded in as props.
//
// Disabled state: button greys out when there's no active plate,
// no objects on the active plate, or no printer bound. Visual
// feedback through opacity + a tooltip naming the blocker.

import { useState } from "react";

import { sliceErrorMessage } from "./reducer";
import type { PlateSnapshot, SceneSnapshot } from "../viewport/types";
import type { JobId, SliceState } from "./types";

/** Why the Slice button can't run right now, or `null` if it can. */
function whyDisabled(
  snapshot: SceneSnapshot | null,
  activePlate: PlateSnapshot | null,
): string | null {
  if (snapshot == null || activePlate == null) {
    return "loading project…";
  }
  if (activePlate.printer_identity == null) {
    return "bind a printer to this plate first";
  }
  if (activePlate.objects.length === 0) {
    return "add an object before slicing";
  }
  return null;
}

export interface SlicePanelProps {
  snapshot: SceneSnapshot | null;
  activePlate: PlateSnapshot | null;
  /** Live slice-job state, owned by `App` and shared with the
   *  `SlicingWindow`. */
  state: SliceState;
  start: () => Promise<JobId>;
  cancel: () => Promise<void>;
}

export function SlicePanel({
  snapshot,
  activePlate,
  state,
  start,
  cancel,
}: SlicePanelProps) {
  const [busy, setBusy] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);

  const inFlight =
    state.status === "running" ||
    state.status === "starting" ||
    state.status === "cancelling";

  const disabledReason = whyDisabled(snapshot, activePlate);
  const sliceDisabled = disabledReason != null || busy;

  async function doSlice() {
    if (disabledReason != null) return;
    setStartError(null);
    setBusy(true);
    try {
      await start();
    } catch (err) {
      setStartError(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function doCancel() {
    setBusy(true);
    try {
      await cancel();
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex items-center gap-2">
      {!inFlight && (
        <button
          type="button"
          onClick={() => void doSlice()}
          disabled={sliceDisabled}
          className="tb-btn primary"
          title={disabledReason ?? "Slice the active plate"}
        >
          Slice
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden>
            <path d="M3 3l6 3-6 3V3z" fill="currentColor" />
          </svg>
        </button>
      )}
      {inFlight && (
        <button
          type="button"
          onClick={() => void doCancel()}
          disabled={state.status === "cancelling" || busy}
          className="tb-btn"
        >
          {state.status === "cancelling" ? "Cancelling…" : "Cancel"}
        </button>
      )}
      {state.status === "failed" && state.error && (
        <span className="text-xs text-rose-400" role="alert">
          {sliceErrorMessage(state.error)}
        </span>
      )}
      {startError && (
        <span className="text-xs text-rose-400" role="alert">
          {startError}
        </span>
      )}
    </div>
  );
}
