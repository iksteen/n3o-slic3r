// Per-plate "where did the most recent slice land" tracker
// (PR-7a-7 wiring).
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
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { SliceEvent } from "./types";

export interface UseLastSliceOutputResult {
  /** Path of the most recent slice's `.gcode` output for the
   * given plate, or `null` if the plate hasn't been sliced this
   * session. */
  pathForPlate(plateId: number): string | null;
}

export function useLastSliceOutput(): UseLastSliceOutputResult {
  const [paths, setPaths] = useState<Record<number, string>>({});

  useEffect(() => {
    let mounted = true;
    let unlisten: UnlistenFn | null = null;

    void (async () => {
      const un = await listen<SliceEvent>("slice:plate_finished", (e) => {
        if (e.payload.kind !== "PlateFinished") return;
        const { plate_id, output_path } = e.payload.data;
        setPaths((prev) => ({ ...prev, [plate_id]: output_path }));
      });
      if (mounted) {
        unlisten = un;
      } else {
        un();
      }
    })();

    return () => {
      mounted = false;
      if (unlisten) unlisten();
    };
  }, []);

  return {
    pathForPlate: (plateId) => paths[plateId] ?? null,
  };
}
