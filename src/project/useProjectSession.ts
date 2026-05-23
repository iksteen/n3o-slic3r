// `useProjectSession` — App.tsx's top-level project state hook
// (PR-5-9).
//
// Owns three pieces of "current session" state the SettingsPanel
// host needs:
//   - `cascadeHandle` — from `cascade_load_default()` on mount.
//   - `printer` — the canonical `PrinterProfileJson` returned by
//     `scene_load_default_printer()` on mount. The bundled A1 mini
//     fixture for now; Phase 9 swaps for a registry.
//   - `snapshot` — the current `SceneSnapshot` (refetched on every
//     scene/project event).
//
// The hook performs the bootstrap dance once on mount, then
// listens for the full firehose of project / scene events so the
// snapshot stays fresh. Project-level fields (cascade handle,
// printer profile) don't change after bootstrap; only the snapshot
// does. We co-locate them all so `App.tsx` reads `useProjectSession()`
// once and gets everything the SettingsPanel host wants.
//
// PR-5-9 also relies on the PlateTabs strip — that already maintains
// its own light-weight view-model via `usePlateTabs`. The two are
// not consolidated because they have different refresh sensitivities
// (the tab strip cares about `object_added`; the cascade context
// doesn't), and keeping their fetches independent costs us at most
// one redundant snapshot per event.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { SceneSnapshot } from "../viewport/types";
import type { PrinterProfileJson } from "../settings/resolve";

/** Events worth refetching the snapshot on. Broader than the tab
 * strip's set — the panel reads selection + overrides + bindings,
 * any of which can shift on a per-plate event. Kept exported so
 * tests can pin the set. */
export const SESSION_EVENT_NAMES = [
  "scene:plate_added",
  "scene:plate_removed",
  "scene:active_plate_changed",
  "scene:plate_metadata_changed",
  "scene:material_binding_changed",
  "scene:object_added",
  "scene:object_removed",
  "scene:object_updated",
  "scene:selection_changed",
  "scene:object_overrides_changed",
  "scene:project_overrides_changed",
  "scene:bed_changed",
  "project:loaded",
] as const;

export interface ProjectSession {
  cascadeHandle: number | null;
  printer: PrinterProfileJson | null;
  snapshot: SceneSnapshot | null;
  /** True until both bootstrap calls return + the first snapshot
   * lands. App-level chrome can render a tiny loading state if it
   * wants. */
  loading: boolean;
  /** Bootstrap error message — non-null indicates the session
   * couldn't initialize (typically: cascade parse failure on a
   * broken bundled file). Surfaces in App.tsx as a banner; the
   * panel won't render in this state. */
  error: string | null;
}

export function useProjectSession(): ProjectSession {
  const [cascadeHandle, setCascadeHandle] = useState<number | null>(null);
  const [printer, setPrinter] = useState<PrinterProfileJson | null>(null);
  const [snapshot, setSnapshot] = useState<SceneSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refetchSnapshot = useCallback(async () => {
    try {
      const snap = await invoke<SceneSnapshot>("scene_snapshot");
      setSnapshot(snap);
    } catch (err) {
      console.error("[session] scene_snapshot failed", err);
    }
  }, []);

  useEffect(() => {
    let mounted = true;
    const unlisteners: UnlistenFn[] = [];

    void (async () => {
      // Subscribe first so an event mid-bootstrap doesn't get
      // dropped on the floor (the worst case is one redundant
      // refetch).
      for (const name of SESSION_EVENT_NAMES) {
        const un = await listen(name, () => {
          void refetchSnapshot();
        });
        if (!mounted) {
          un();
          continue;
        }
        unlisteners.push(un);
      }

      // Bootstrap: cascade + default printer + first snapshot in
      // parallel. Failures are isolated so a missing default printer
      // doesn't take out cascade load.
      try {
        const [handle, prof] = await Promise.all([
          invoke<number>("cascade_load_default"),
          invoke<PrinterProfileJson>("scene_load_default_printer"),
        ]);
        if (!mounted) return;
        setCascadeHandle(handle);
        setPrinter(prof);
        await refetchSnapshot();
      } catch (err) {
        if (!mounted) return;
        setError(String(err));
        console.error("[session] bootstrap failed", err);
      } finally {
        if (mounted) setLoading(false);
      }
    })();

    return () => {
      mounted = false;
      for (const un of unlisteners) un();
    };
  }, [refetchSnapshot]);

  return { cascadeHandle, printer, snapshot, loading, error };
}
