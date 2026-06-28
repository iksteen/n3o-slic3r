// The `scene_snapshot` query — the shared source the project mirror hooks read.
//
// `scene_snapshot` is the single hottest backend read in the app: the settings
// host, the plate tab strip, and (eventually) the objects panel all pull it
// and refetch on the same scene/project events. Defining it once here means
// every consumer shares ONE fetch + ONE invalidation set via the query cache,
// instead of each hook re-invoking the command on every overlapping event.
//
// The event set is the SUPERSET any consumer needs (the tab strip watches a
// subset). A projection-only consumer should read this through a memoized
// selector so the extra invalidations it doesn't care about don't re-render it
// — see the note in queryCache.ts.

import { invoke } from "@tauri-apps/api/core";
import type { SceneSnapshot } from "../viewport/types";
import { defineQuery } from "./queryCache";

/** Every event that can change the snapshot a panel/strip reads: plate
 *  lifecycle, active-plate navigation, per-plate content + overrides, the
 *  project-wide override tier, and whole-session replacement (load/save-as/
 *  import). Kept exported so tests can pin the set. */
export const SCENE_SNAPSHOT_EVENTS = [
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
  "scene:user_overrides_changed",
  "scene:bed_changed",
  "project:loaded",
  // Save-as changes the project's source_path; refetch so the File menu's
  // filename label updates. (Plain saves re-emit it harmlessly.)
  "project:saved",
  // Importing a foreign project replaces the whole session.
  "project:imported",
  // Undo/redo swaps the live project wholesale — resync like a load.
  "project:restored",
] as const;

export const sceneSnapshotQuery = defineQuery<SceneSnapshot>({
  key: "scene_snapshot",
  fetch: () => invoke<SceneSnapshot>("scene_snapshot"),
  invalidateOn: SCENE_SNAPSHOT_EVENTS,
});
