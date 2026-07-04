// `usePlateTabs` — the tab strip's data hook.
//
// State-layer spike: the strip reads the shared `scene_snapshot` query (the
// same one `useProjectSession` reads) instead of running its own invoke +
// listen loop. The two now share ONE fetch and ONE invalidation set — a
// `scene:object_added` triggers a single shared refetch, not one per hook.
//
// The strip only needs a projection (id / name / printerLabel / objectCount +
// the active id), so it reads through `useQuerySelector`: the shared query
// invalidates on its full superset, but the strip re-renders only when its
// projected slice actually changes. So events the projection ignores (e.g.
// `scene:selection_changed`, an object transform that leaves the count alone)
// cost at most one shared fetch and zero strip re-renders — strictly better
// than the old per-hook subscription, never worse.
//
// `PLATE_TAB_EVENT_NAMES` documents the events whose effect the projection
// reflects; they're a subset of the shared query's invalidation set.

import type { PlateId, SceneSnapshot } from "../viewport/types";
import { useQuerySelector, type QueryState } from "../state/queryCache";
import { sceneSnapshotQuery } from "../state/sceneSnapshot";

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
  "scene:plate_changed",
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

/** Select the strip's view-model from the shared query's state. Before the
 *  first snapshot lands there are no tabs — render the empty skeleton. */
function selectTabs(s: QueryState<SceneSnapshot>): PlateTabsState {
  if (s.data == null) {
    return { plates: [], activePlateId: null, loading: s.loading };
  }
  return projectSnapshot(s.data);
}

/** Structural equality over the projected view-model — lets `useQuerySelector`
 *  skip a re-render when a shared-query refetch leaves the strip's slice
 *  unchanged (e.g. a selection change, or an object transform that doesn't
 *  alter the per-tab object count). */
function tabsEqual(a: PlateTabsState, b: PlateTabsState): boolean {
  if (a.loading !== b.loading) return false;
  if (a.activePlateId !== b.activePlateId) return false;
  if (a.plates.length !== b.plates.length) return false;
  for (let i = 0; i < a.plates.length; i++) {
    const x = a.plates[i];
    const y = b.plates[i];
    if (
      x.id !== y.id ||
      x.name !== y.name ||
      x.printerLabel !== y.printerLabel ||
      x.objectCount !== y.objectCount
    ) {
      return false;
    }
  }
  return true;
}

export function usePlateTabs(): PlateTabsState {
  return useQuerySelector(sceneSnapshotQuery, selectTabs, tabsEqual);
}
