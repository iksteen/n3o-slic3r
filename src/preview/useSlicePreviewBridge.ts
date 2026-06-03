// Bridges the slice-job event channel to the preview pipeline
// (PR-6-15).
//
// Per-plate handle cache so plate-switching while in preview
// mode flips to the cached preview instead of re-loading. Also
// listens for the `slice:plate_finished` event to auto-load the
// gcode and fire `onPreviewReady` so App can switch to preview.

import { useCallback, useEffect, useRef, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { onEvents } from "../state/eventRouter";

import { previewDrop, previewLoad } from "./invokes";
import type { PreviewLoadResponse } from "./types";
import { listenPlateEdits } from "../project/editEvents";

interface PlateFinishedPayload {
  data?: {
    job_id?: number;
    plate_id?: number;
    output_path?: string;
  };
}

export interface SlicePreviewBridge {
  /** Preview-load response for the active plate. `null` when
   * no slice has finished for the active plate yet. */
  activePreview: PreviewLoadResponse | null;
  /** Fires when a fresh preview lands, with the plate it was sliced
   * for. App.tsx flips `mode = "preview"` from this — but only when
   * that plate is the active one, so a slice finishing for a tab the
   * user has navigated away from doesn't yank the view. */
  onPreviewReady: (callback: (plateId: number) => void) => void;
  /** Drop the cached preview for a given plate. Called when
   * removing a plate from the project. */
  forgetPlate: (plateId: number) => void;
}

/** Hook owning the slice→preview bridge state for the current
 * App session. Pass `activePlateId` so the bridge knows which
 * cached preview to surface. */
export function useSlicePreviewBridge(
  activePlateId: number | null,
): SlicePreviewBridge {
  const cacheRef = useRef<Map<number, PreviewLoadResponse>>(new Map());
  const [tick, setTick] = useState(0); // force re-render when cache mutates

  const readyCallbackRef = useRef<((plateId: number) => void) | null>(null);

  useEffect(() => {
    const off = onEvents<PlateFinishedPayload>(
      ["slice:plate_finished"],
      (e) => {
        const data = e.payload?.data;
        if (!data?.plate_id || !data?.output_path) return;
        const plateId = data.plate_id;
        const path = data.output_path;
        // Load the gcode through the preview pipeline. Drop the previous
        // cached preview for the same plate so we don't leak the
        // 250MB-per-load memory.
        const prior = cacheRef.current.get(plateId);
        if (prior) {
          void previewDrop(prior.handle).catch(() => undefined);
        }
        void previewLoad(path)
          .then((res) => {
            cacheRef.current.set(plateId, res);
            setTick((t) => t + 1);
            if (readyCallbackRef.current) {
              readyCallbackRef.current(plateId);
            }
          })
          .catch((err) =>
            console.error("[preview] auto-load after slice failed", err),
          );
      },
    );
    return () => {
      off();
      // Drop all cached previews on unmount.
      for (const r of cacheRef.current.values()) {
        void previewDrop(r.handle).catch(() => undefined);
      }
      cacheRef.current.clear();
    };
  }, []);

  // Editing a plate makes its preview stale — blank it (drop the cached
  // handle so the workspace falls back to its "slice to preview" state and a
  // stale render can't be shown or sent). A project-wide edit (user
  // overrides) blanks every plate's preview.
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    const dropPlate = (plateId: number) => {
      const prior = cacheRef.current.get(plateId);
      if (prior) {
        void previewDrop(prior.handle).catch(() => undefined);
        cacheRef.current.delete(plateId);
        setTick((t) => t + 1);
      }
    };
    void (async () => {
      unlisten = await listenPlateEdits(dropPlate, () => {
        if (cacheRef.current.size === 0) return;
        for (const r of cacheRef.current.values()) {
          void previewDrop(r.handle).catch(() => undefined);
        }
        cacheRef.current.clear();
        setTick((t) => t + 1);
      });
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  // tick is consumed implicitly via the closure below — keep it
  // referenced so React's hook deps system doesn't complain.
  void tick;

  const activePreview =
    activePlateId != null ? cacheRef.current.get(activePlateId) ?? null : null;

  const onPreviewReady = useCallback(
    (callback: (plateId: number) => void) => {
      readyCallbackRef.current = callback;
    },
    [],
  );

  const forgetPlate = useCallback((plateId: number) => {
    const prior = cacheRef.current.get(plateId);
    if (prior) {
      void previewDrop(prior.handle).catch(() => undefined);
      cacheRef.current.delete(plateId);
      setTick((t) => t + 1);
    }
  }, []);

  return { activePreview, onPreviewReady, forgetPlate };
}
