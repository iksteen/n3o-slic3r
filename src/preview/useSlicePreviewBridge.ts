// Bridges the slice-job event channel to the preview pipeline
// (PR-6-15).
//
// Per-plate handle cache so plate-switching while in preview
// mode flips to the cached preview instead of re-loading. Also
// listens for the `slice:plate_finished` event to auto-load the
// gcode + (optionally) auto-switch the App's mode.

import { useCallback, useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { previewDrop, previewLoad } from "./invokes";
import type { PreviewLoadResponse } from "./types";

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
  /** Auto-switch the App's mode to preview when the slice
   * finishes. Set by App.tsx based on whether the user has
   * manually toggled out of preview during this run. */
  enableAutoSwitch: (enabled: boolean) => void;
  /** Fires when a fresh preview lands. App.tsx flips
   * `mode = "preview"` from this when auto-switch is enabled. */
  onPreviewReady: (callback: () => void) => void;
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

  const autoSwitchRef = useRef<boolean>(true);
  const readyCallbackRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    void (async () => {
      unlisten = await listen<PlateFinishedPayload>(
        "slice:plate_finished",
        (e) => {
          const data = e.payload?.data;
          if (!data?.plate_id || !data?.output_path) return;
          const plateId = data.plate_id;
          const path = data.output_path;
          // Load the gcode through the preview pipeline. Drop
          // the previous cached preview for the same plate so
          // we don't leak the 250MB-per-load memory.
          const prior = cacheRef.current.get(plateId);
          if (prior) {
            void previewDrop(prior.handle).catch(() => undefined);
          }
          void previewLoad(path)
            .then((res) => {
              cacheRef.current.set(plateId, res);
              setTick((t) => t + 1);
              if (autoSwitchRef.current && readyCallbackRef.current) {
                readyCallbackRef.current();
              }
            })
            .catch((err) =>
              console.error("[preview] auto-load after slice failed", err),
            );
        },
      );
    })();
    return () => {
      if (unlisten) unlisten();
      // Drop all cached previews on unmount.
      for (const r of cacheRef.current.values()) {
        void previewDrop(r.handle).catch(() => undefined);
      }
      cacheRef.current.clear();
    };
  }, []);

  // tick is consumed implicitly via the closure below — keep it
  // referenced so React's hook deps system doesn't complain.
  void tick;

  const activePreview =
    activePlateId != null ? cacheRef.current.get(activePlateId) ?? null : null;

  const enableAutoSwitch = useCallback((enabled: boolean) => {
    autoSwitchRef.current = enabled;
  }, []);

  const onPreviewReady = useCallback((callback: () => void) => {
    readyCallbackRef.current = callback;
  }, []);

  const forgetPlate = useCallback((plateId: number) => {
    const prior = cacheRef.current.get(plateId);
    if (prior) {
      void previewDrop(prior.handle).catch(() => undefined);
      cacheRef.current.delete(plateId);
      setTick((t) => t + 1);
    }
  }, []);

  return { activePreview, enableAutoSwitch, onPreviewReady, forgetPlate };
}
