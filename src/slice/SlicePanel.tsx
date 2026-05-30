// Slice button + in-flight progress bar (PR-3-4, rewired in PR-6-3).
// Post-slice stats (time / filament / layers) and the clear button
// were dropped from the header — the slice result surfaces in Preview.
//
// Post-PR-6-3 the Slice button drives off live project state via
// `slice_active_plate` — no file picker, no model-path tracking.
// The backend builds the SliceJobInput from the active plate's
// scene + bindings + overrides and writes its own temp .3mf for
// libslic3r to load. Output gcode lands in a per-job temp dir;
// the path appears on `slice:plate_finished` events.
//
// Disabled state: button greys out when there's no active plate,
// no objects on the active plate, or no printer bound. Visual
// feedback through opacity + a tooltip naming the blocker.

import { useState } from "react";

import { sliceErrorMessage } from "./reducer";
import type { PlateSnapshot, SceneSnapshot } from "../viewport/types";
import { useSliceJob } from "./useSliceJob";

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
}

export function SlicePanel({ snapshot, activePlate }: SlicePanelProps) {
  const { state, start, cancel } = useSliceJob();
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
          className="px-2 py-1 bg-emerald-700 hover:bg-emerald-600 disabled:opacity-40 rounded text-xs font-medium"
          title={disabledReason ?? "Slice the active plate"}
        >
          Slice
        </button>
      )}
      {inFlight && (
        <button
          type="button"
          onClick={() => void doCancel()}
          disabled={state.status === "cancelling" || busy}
          className="px-2 py-1 bg-rose-700 hover:bg-rose-600 disabled:opacity-40 rounded text-xs font-medium"
        >
          {state.status === "cancelling" ? "Cancelling…" : "Cancel"}
        </button>
      )}
      {inFlight && (
        <div className="flex items-center gap-2 min-w-[14rem]">
          <div className="flex-1 h-1.5 bg-neutral-800 rounded overflow-hidden">
            <div
              className="h-full bg-emerald-500 transition-[width] duration-150"
              style={{
                width: `${Math.max(0, Math.min(100, state.percent))}%`,
              }}
            />
          </div>
          <span className="text-xs text-neutral-300 font-mono w-10 text-right">
            {state.percent}%
          </span>
          <span className="text-xs text-neutral-400">
            {state.status === "starting"
              ? "starting…"
              : state.status === "cancelling"
                ? `cancelling (plate ${state.plate_id ?? "?"})`
                : `plate ${state.plate_id ?? "?"} · ${state.stage || "…"}`}
          </span>
        </div>
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
