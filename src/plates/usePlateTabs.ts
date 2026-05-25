// `usePlateTabs` — the tab strip's data hook.
//
// Fetches `scene_snapshot` for the initial state, then listens to
// the four plate-affecting Tauri events and re-fetches the
// snapshot to refresh the strip's view-model. We're not piggybacking
// on `SceneMirror` because:
//   - The mirror is owned by `ViewportCanvas`; the tab strip
//     mounts at the App level a layer above it.
//   - The strip needs per-plate object counts + printer label —
//     fields the snapshot carries directly. Snapshot fetches are
//     cheap (headers + metadata only, no mesh buffers) so simple
//     re-fetch on event is correct and easy to reason about.
//
// The events watched cover every state change the strip cares
// about:
//   - `scene:plate_added` / `scene:plate_removed` — strip layout
//   - `scene:active_plate_changed` — highlight
//   - `scene:plate_metadata_changed` — name (PR-5-3 rename) /
//     cycle count (PR-5-5) / composition order (PR-5-5)
//   - `scene:object_added` / `scene:object_removed` — per-tab
//     object count
//   - `scene:bed_changed` — printer label changes when a plate's
//     printer binding updates (the strip shows the printer
//     identity from `plate.printer`)
//   - `project:loaded` — wholesale state replacement
//
// The strip view-model is intentionally a minimal projection of
// `SceneSnapshot`; we don't expose the raw snapshot to the tab
// component because that would couple it to fields it has no
// business reading.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { PlateId, SceneSnapshot } from "../viewport/types";

/** What one tab needs to render. Names match the design's
 * `plate.{id,name,printer,objects}` access pattern (the design
 * uses `objects.length` — we precompute that as `objectCount`). */
export interface PlateTabView {
  id: PlateId;
  name: string;
  /** Display label for the plate's bound printer. `null` when the
   * plate hasn't been assigned a printer (the design renders an
   * "—" placeholder in that case). */
  printerLabel: string | null;
  objectCount: number;
}

export interface PlateTabsState {
  plates: PlateTabView[];
  activePlateId: PlateId | null;
  /** True until the first snapshot lands. The strip renders an
   * empty skeleton in this window — no plates yet means no tabs. */
  loading: boolean;
}

/** Events that should trigger a snapshot re-fetch. Listed as a
 * constant so test code can iterate the same set the hook listens
 * to. */
export const PLATE_TAB_EVENT_NAMES = [
  "scene:plate_added",
  "scene:plate_removed",
  "scene:active_plate_changed",
  "scene:plate_metadata_changed",
  "scene:object_added",
  "scene:object_removed",
  "scene:bed_changed",
  "project:loaded",
] as const;

/** Project a full `SceneSnapshot` down to the tab-strip's
 * view-model. Exposed for tests so they don't have to spin up the
 * full hook to exercise the projection. */
export function projectSnapshot(snap: SceneSnapshot): PlateTabsState {
  return {
    plates: snap.plates.map((p) => ({
      id: p.plate_id,
      name: p.name,
      printerLabel: p.printer_identity ?? null,
      objectCount: p.objects.length,
    })),
    activePlateId: snap.active_plate_id,
    loading: false,
  };
}

export function usePlateTabs(): PlateTabsState {
  const [state, setState] = useState<PlateTabsState>({
    plates: [],
    activePlateId: null,
    loading: true,
  });

  const refetch = useCallback(async () => {
    try {
      const snap = await invoke<SceneSnapshot>("scene_snapshot");
      setState(projectSnapshot(snap));
    } catch (err) {
      console.error("[plates] scene_snapshot failed", err);
    }
  }, []);

  useEffect(() => {
    let mounted = true;
    const unlisteners: UnlistenFn[] = [];

    void (async () => {
      // Subscribe before the initial fetch so an event that races
      // the snapshot can't be lost (worst case: it triggers a
      // redundant re-fetch).
      for (const name of PLATE_TAB_EVENT_NAMES) {
        const un = await listen(name, () => {
          void refetch();
        });
        if (!mounted) {
          un();
          continue;
        }
        unlisteners.push(un);
      }
      await refetch();
    })();

    return () => {
      mounted = false;
      for (const un of unlisteners) un();
    };
  }, [refetch]);

  return state;
}
