// Slice / Cancel button for the topbar.
// Post-slice stats (time / filament / layers) and the clear button
// were dropped from the header — the slice result surfaces in Preview.
// In-flight progress (and the Cancel control) moved out of the topbar
// into the floating `SliceProgressWindow` over the canvas (design's
// `.slicing-window`); this panel is now just the Slice button, which
// disables while a slice runs rather than swapping to a Cancel button.
//
// The Slice button drives off live project state via
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

import type { PlateSnapshot, SceneSnapshot } from "../viewport/types";
import type { JobId, SliceState } from "./types";
import { pushLog } from "../logging/logStore";

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
   *  `SliceProgressWindow`. */
  state: SliceState;
  start: () => Promise<JobId>;
}

export function SlicePanel({
  snapshot,
  activePlate,
  state,
  start,
}: SlicePanelProps) {
  const [busy, setBusy] = useState(false);

  const inFlight =
    state.status === "running" ||
    state.status === "starting" ||
    state.status === "cancelling";

  const disabledReason = whyDisabled(snapshot, activePlate);

  async function doSlice() {
    if (disabledReason != null) return;
    setBusy(true);
    try {
      await start();
    } catch (err) {
      // Route to the error console (which pops open) like an async slice
      // failure does — not inline into the topbar.
      pushLog("error", `Slice failed to start: ${String(err)}`);
    } finally {
      setBusy(false);
    }
  }

  // The button stays in place and just disables while a slice runs — the
  // Cancel control lives in the floating SliceProgressWindow.
  return (
    <div className="flex items-center gap-2">
      <button
        type="button"
        onClick={() => void doSlice()}
        disabled={disabledReason != null || busy || inFlight}
        className="tb-btn primary"
        title={
          inFlight
            ? "Slicing… (cancel in the slice window)"
            : (disabledReason ?? "Slice the active plate")
        }
      >
        Slice
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden>
          <path d="M3 3l6 3-6 3V3z" fill="currentColor" />
        </svg>
      </button>
    </div>
  );
}
