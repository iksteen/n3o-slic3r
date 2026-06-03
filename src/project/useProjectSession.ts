// `useProjectSession` — App.tsx's top-level project state hook (PR-5-9).
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
import type { SceneSnapshot } from "../viewport/types";
import { useQuery } from "../state/queryCache";
import { onEvents } from "../state/eventRouter";
import { sceneSnapshotQuery, SCENE_SNAPSHOT_EVENTS } from "../state/sceneSnapshot";
import { isEditEvent, isSavedEvent } from "./editEvents";

/** Back-compat re-export: the event set the session refetches on now lives
 *  with the `scene_snapshot` query. Kept under the old name so existing
 *  importers (and the host's floor-pinning test) don't churn. */
export const SESSION_EVENT_NAMES = SCENE_SNAPSHOT_EVENTS;

export interface ProjectSession {
  /** Always `null` post-PR-S-5c. Kept on the interface so the SettingsPanel
   * host can pass it through without conditionalizing its prop shape. */
  cascadeHandle: number | null;
  snapshot: SceneSnapshot | null;
  /** True when the project has unsaved edits — set by any content edit,
   * cleared on save / load / import. Drives the title-bar unsaved marker. */
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

  // Dirty tracking rides the shared router: a content edit dirties the
  // project; save/load/import returns it to a clean baseline. (Selection +
  // navigation aren't edits — see editEvents.) Same event names as the query,
  // so this reuses the router's per-name Tauri subscriptions.
  useEffect(() => {
    return onEvents(SCENE_SNAPSHOT_EVENTS, (event) => {
      const name = event.event;
      if (isSavedEvent(name)) setDirty(false);
      else if (isEditEvent(name)) setDirty(true);
    });
  }, []);

  return { cascadeHandle: null, snapshot, dirty, loading, error };
}
