// Which scene events count as an EDIT — i.e. change a plate's slicable
// content. Used to invalidate per-plate slice artifacts:
//   - slice-output invalidation (edit → drop the plate's last slice so Send
//     can't push a stale gcode; useLastSliceOutput)
//   - preview invalidation (edit → blank the plate's preview;
//     useSlicePreviewBridge)
//
// Dirty/unsaved tracking is NOT here — that's backend-authoritative now
// (`DirtyTracker` + `project:dirty_changed`; useProjectSession reads it).
//
// Deliberately EXCLUDES non-edits: selection (`scene:selection_changed`),
// navigation (`scene:active_plate_changed`), structural add/remove of empty
// plates, and the warning events (out-of-bounds, …).

import type { UnlistenFn } from "@tauri-apps/api/event";
import { onEvents } from "../state/eventRouter";

/** Plate-scoped content edits — each carries `data.plate_id` and invalidates
 *  just that plate. */
export const PLATE_EDIT_EVENTS = [
  "scene:object_added",
  "scene:object_updated",
  "scene:object_removed",
  "scene:material_slot_changed",
  "scene:object_overrides_changed",
  "scene:group_overrides_changed",
  "scene:project_overrides_changed",
  "scene:plate_changed",
  "scene:bed_changed",
] as const;

/** Project-wide edit (user-tier overrides apply to every plate's slice), so
 *  it invalidates ALL plates. */
export const PROJECT_WIDE_EDIT_EVENT = "scene:user_overrides_changed";

/** Wholesale project replacement — Open project (native load or transparent
 *  foreign import). Not an *edit* (the in-memory project is swapped out, not
 *  mutated), but every per-plate slice artifact keyed by plate id — last-slice
 *  output, G-code preview, priming-tower mesh — is now stale (plate ids are
 *  reused across projects, so a cache entry for "plate 1" would otherwise show
 *  the previous project's slice). So it invalidates ALL plates, routed through
 *  `listenPlateEdits`' `onAll`. `project:saved` is deliberately excluded:
 *  saving doesn't change geometry, the slice stays valid. */
export const PROJECT_REPLACED_EVENTS = [
  "project:loaded",
  "project:imported",
  // Undo/redo swaps the live project, so every per-plate slice artifact
  // (output, preview, tower) is stale — same as a load.
  "project:restored",
] as const;

interface EditPayload {
  data?: { plate_id?: number };
}

/** Subscribe to content edits via the shared event router. `onPlate(plateId)`
 *  fires for a plate-scoped edit; `onAll()` for a project-wide one — a
 *  project-wide override edit OR a wholesale project replacement (Open /
 *  import), both of which stale every plate. Returns a single unsubscribe.
 *  Synchronous — the router registration is synchronous, so callers can use
 *  it directly as an effect body. */
export function listenPlateEdits(
  onPlate: (plateId: number) => void,
  onAll: () => void,
): UnlistenFn {
  const offPlate = onEvents<EditPayload>(PLATE_EDIT_EVENTS, (e) => {
    const plateId = e.payload?.data?.plate_id;
    if (plateId != null) onPlate(plateId);
  });
  const offAll = onEvents([PROJECT_WIDE_EDIT_EVENT], () => onAll());
  const offReplaced = onEvents(PROJECT_REPLACED_EVENTS, () => onAll());
  return () => {
    offPlate();
    offAll();
    offReplaced();
  };
}
