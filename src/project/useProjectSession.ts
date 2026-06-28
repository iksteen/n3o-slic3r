// `useProjectSession` — App.tsx's top-level project state hook.
//
// Owns the "current session" the SettingsPanel host needs: the current
// `SceneSnapshot` plus the unsaved-edits `dirty` flag.
//
// State-layer spike: the snapshot now comes from the shared `scene_snapshot`
// query (src/state) instead of this hook's own invoke + listen loop. Any other
// consumer of that query (e.g. usePlateTabs, once converted) shares the same
// fetch and the same invalidation set — one `scene:object_added` triggers one
// refetch, not one per hook. The `dirty` flag stays here (it's session-derived
// client state, not a backend value) but rides the same shared event router,
// so it adds classification, not another batch of Tauri subscriptions.
//
// The active printer profile is NOT held here. The host derives it from the
// active plate's `printer_identity` against the printer catalog.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SceneSnapshot } from "../viewport/types";
import { useQuery } from "../state/queryCache";
import { onEvents } from "../state/eventRouter";
import { sceneSnapshotQuery, SCENE_SNAPSHOT_EVENTS } from "../state/sceneSnapshot";

/** Back-compat re-export: the event set the session refetches on now lives
 *  with the `scene_snapshot` query. Kept under the old name so existing
 *  importers (and the host's floor-pinning test) don't churn. */
export const SESSION_EVENT_NAMES = SCENE_SNAPSHOT_EVENTS;

export interface ProjectSession {
  /** Always `null` (the cascade handle was retired). Kept on the interface so
   * the SettingsPanel host can pass it through without conditionalizing its prop shape. */
  cascadeHandle: number | null;
  snapshot: SceneSnapshot | null;
  /** True when the project has unsaved edits. Backend-authoritative
   * (`DirtyTracker`): set by any content edit, cleared on save / load /
   * import. Drives the title-bar unsaved marker. */
  dirty: boolean;
  /** True until the first snapshot lands. */
  loading: boolean;
  /** Bootstrap error message — non-null indicates the session couldn't
   * initialize. Surfaces in App.tsx as a banner. */
  error: string | null;
}

export function useProjectSession(): ProjectSession {
  const { data: snapshot, loading, error } = useQuery(sceneSnapshotQuery);
  const [dirty, setDirty] = useState(false);

  // Dirty state is owned by the backend `DirtyTracker` (the same edit
  // classification that gates autosave). Read it once on mount, then track
  // `project:dirty_changed` — emitted only when the flag flips.
  useEffect(() => {
    let active = true;
    invoke<boolean>("project_is_dirty")
      .then((d) => {
        if (active) setDirty(d);
      })
      .catch(() => {});
    const off = onEvents<{ dirty: boolean }>(
      ["project:dirty_changed"],
      (event) => setDirty(event.payload.dirty),
    );
    return () => {
      active = false;
      off();
    };
  }, []);

  return { cascadeHandle: null, snapshot, dirty, loading, error };
}
