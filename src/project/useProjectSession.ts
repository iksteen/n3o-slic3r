// `useProjectSession` — App.tsx's top-level project state hook
// (PR-5-9).
//
// Owns two pieces of "current session" state the SettingsPanel
// host needs:
//   - `printer` — the canonical `PrinterProfileJson` returned by
//     `scene_load_default_printer()` on mount. The bundled A1 mini
//     fixture for now; Phase 9 swaps for a registry.
//   - `snapshot` — the current `SceneSnapshot` (refetched on every
//     scene/project event).
//
// `cascadeHandle` is no longer plumbed here — the slice path
// composes the cascade fresh per job from the bound printer
// instance (PR-S-5c), and the SettingsPanel's resolved-value display
// is wired separately downstream.
//
// The hook performs the bootstrap dance once on mount, then
// listens for the full firehose of project / scene events so the
// snapshot stays fresh.

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
  "scene:material_slot_changed",
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
  /** Always `null` post-PR-S-5c. Kept on the interface so the
   * SettingsPanel host can pass it through without conditionalizing
   * its prop shape; downstream consumers treat `null` as "no
   * resolved values to render" the same way they did with the
   * legacy missing-handle case. */
  cascadeHandle: number | null;
  printer: PrinterProfileJson | null;
  snapshot: SceneSnapshot | null;
  /** True until the bootstrap call returns + the first snapshot
   * lands. App-level chrome can render a tiny loading state if it
   * wants. */
  loading: boolean;
  /** Bootstrap error message — non-null indicates the session
   * couldn't initialize. Surfaces in App.tsx as a banner; the
   * panel won't render in this state. */
  error: string | null;
}

export function useProjectSession(): ProjectSession {
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

      try {
        const prof = await invoke<PrinterProfileJson>(
          "scene_load_default_printer",
        );
        if (!mounted) return;
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

  return { cascadeHandle: null, printer, snapshot, loading, error };
}
