// Slice button + progress bar + per-plate summary cards (PR-3-4).
//
// Minimum-viable surface to drive the slice loop; visuals follow
// Phase 2's neutral-900 dark palette and Phase 4 will restyle.
// Self-contained: the user picks a model file via the bundled
// `tauri-plugin-dialog`, the panel maps the basename onto a
// per-job temp directory and shows progress + summary inline in the
// header row alongside the cancel button.

import { useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { sliceErrorMessage } from "./reducer";
import type { PlateSummary } from "./types";
import { useSliceJob } from "./useSliceJob";

function formatDuration(seconds: number): string {
  if (seconds <= 0) return "—";
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = Math.floor(seconds % 60);
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${secs}s`;
  return `${secs}s`;
}

function summarizeFilament(summary: PlateSummary): string {
  // Aggregate across extruders; per-extruder breakdown lands when
  // multi-tool / AMS UI ships in Phase 5.
  let grams = 0;
  let mm = 0;
  for (const v of Object.values(summary.filament_used_grams)) grams += v;
  for (const v of Object.values(summary.filament_used_mm)) mm += v;
  if (grams === 0 && mm === 0) return "—";
  return `${grams.toFixed(1)}g · ${(mm / 1000).toFixed(2)}m`;
}

export function SlicePanel() {
  const { state, start, cancel, reset } = useSliceJob();
  const [modelPath, setModelPath] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);

  const inFlight =
    state.status === "running" ||
    state.status === "starting" ||
    state.status === "cancelling";

  async function pickModel() {
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: "Mesh", extensions: ["stl", "obj", "3mf"] }],
    });
    if (typeof picked === "string") {
      setModelPath(picked);
      setStartError(null);
    }
  }

  async function doSlice() {
    if (modelPath == null) return;
    setStartError(null);
    setBusy(true);
    try {
      // One job → one temp dir. Backend writes
      // `<output_dir>/plate_1.gcode` per PR-3-2.
      const stamp = Date.now();
      const outputDir = `/tmp/n3o-slice-${stamp}`;
      await start(modelPath, outputDir);
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

  const basename = modelPath != null ? modelPath.split(/[\\/]/).pop() : null;

  return (
    <div className="flex items-center gap-2">
      <button
        type="button"
        onClick={() => void pickModel()}
        disabled={inFlight}
        className="px-2 py-1 bg-neutral-800 hover:bg-neutral-700 disabled:opacity-40 rounded text-xs"
        title={modelPath ?? "Pick a model to slice"}
      >
        {basename ?? "Pick model…"}
      </button>
      {!inFlight && (
        <button
          type="button"
          onClick={() => void doSlice()}
          disabled={modelPath == null || busy}
          className="px-2 py-1 bg-emerald-700 hover:bg-emerald-600 disabled:opacity-40 rounded text-xs font-medium"
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
      {(state.status === "complete" ||
        (state.status === "cancelled" && state.summaries.length > 0)) &&
        state.summaries.map((s, i) => (
          <div
            key={`${s.output_path}-${i}`}
            className="px-2 py-1 bg-neutral-800 rounded text-xs text-neutral-200 flex items-center gap-2"
            title={s.output_path}
          >
            <span className="font-mono">
              {s.estimated_time_text || formatDuration(s.estimated_time_seconds)}
            </span>
            <span className="text-neutral-400">·</span>
            <span>{summarizeFilament(s)}</span>
            <span className="text-neutral-400">·</span>
            <span>{s.layer_count} layers</span>
          </div>
        ))}
      {(state.status === "failed" ||
        state.status === "cancelled" ||
        state.status === "complete") && (
        <button
          type="button"
          onClick={reset}
          className="px-2 py-1 bg-neutral-800 hover:bg-neutral-700 rounded text-xs"
          title="Clear last result"
        >
          Clear
        </button>
      )}
      {startError && (
        <span className="text-xs text-rose-400" role="alert">
          {startError}
        </span>
      )}
    </div>
  );
}
