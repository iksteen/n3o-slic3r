// Per-plate "where did the most recent slice land" tracker.
//
// Listens to `slice:plate_finished` events and remembers the
// `output_path` (a raw `.gcode` file on disk) for each plate
// the user has sliced this session. The printer-panel's Send /
// Dry-run buttons consume this to know what to wrap and upload.
//
// Lives independently of `useSliceJob` so the panel doesn't have
// to hoist the slice-job state into App and re-thread it. Both
// hooks subscribe to the same event channel; that's fine — Tauri
// `listen` returns separate subscriptions and the cost is one
// extra setState per event, which is negligible at typical
// completion rates.

import { useEffect, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { onEvents } from "../state/eventRouter";
import type { SliceEvent } from "./types";
import { listenPlateEdits } from "../project/editEvents";

export interface UseLastSliceOutputResult {
  /** Path of the most recent slice's `.gcode` output for the
   * given plate, or `null` if the plate hasn't been sliced this
   * session. */
  pathForPlate(plateId: number): string | null;
}

export function useLastSliceOutput(): UseLastSliceOutputResult {
  const [paths, setPaths] = useState<Record<number, string>>({});

  useEffect(() => {
    return onEvents<SliceEvent>(
      ["slice:plate_started", "slice:plate_finished"],
      (e) => {
        if (e.payload.kind === "PlateStarted") {
          // A new slice for this plate just began: its prior output is now
          // stale. Drop it immediately so Send/Export gate ("Slice the
          // plate first") until the new slice lands — a re-slice (or one
          // that then fails/cancels) can never push the old gcode.
          const { plate_id } = e.payload.data;
          setPaths((prev) => {
            if (!(plate_id in prev)) return prev;
            const next = { ...prev };
            delete next[plate_id];
            return next;
          });
        } else if (e.payload.kind === "PlateFinished") {
          const { plate_id, output_path } = e.payload.data;
          setPaths((prev) => ({ ...prev, [plate_id]: output_path }));
        }
      },
    );
  }, []);

  // Editing a plate invalidates its last slice: drop the path so Send/Export
  // can't push a gcode that no longer matches the plate (sendGate reports
  // "Slice the plate first" once the path is gone). A project-wide edit
  // (user overrides) invalidates every plate's slice.
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    void (async () => {
      unlisten = await listenPlateEdits(
        (plateId) =>
          setPaths((prev) => {
            if (!(plateId in prev)) return prev;
            const next = { ...prev };
            delete next[plateId];
            return next;
          }),
        () => setPaths((prev) => (Object.keys(prev).length ? {} : prev)),
      );
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  return {
    pathForPlate: (plateId) => paths[plateId] ?? null,
  };
}
