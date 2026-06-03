// Which scene events count as an EDIT — i.e. change a plate's slicable
// content. One classification, used by three consumers:
//   - dirty tracking (any edit → project unsaved; useProjectSession)
//   - slice-output invalidation (edit → drop the plate's last slice so Send
//     can't push a stale gcode; useLastSliceOutput)
//   - preview invalidation (edit → blank the plate's preview;
//     useSlicePreviewBridge)
//
// Deliberately EXCLUDES non-edits: selection (`scene:selection_changed`),
// navigation (`scene:active_plate_changed`), structural add/remove of empty
// plates, and the warning events (out-of-bounds, non-uniform-scale, …).

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Plate-scoped content edits — each carries `data.plate_id` and invalidates
 *  just that plate. */
export const PLATE_EDIT_EVENTS = [
  "scene:object_added",
  "scene:object_updated",
  "scene:object_removed",
  "scene:material_slot_changed",
  "scene:object_overrides_changed",
  "scene:project_overrides_changed",
  "scene:plate_metadata_changed",
  "scene:bed_changed",
] as const;

/** Project-wide edit (user-tier overrides apply to every plate's slice), so
 *  it invalidates ALL plates. */
export const PROJECT_WIDE_EDIT_EVENT = "scene:user_overrides_changed";

/** Lifecycle events that return the project to a saved/clean baseline. */
export const SAVED_EVENTS = [
  "project:saved",
  "project:loaded",
  "project:imported",
] as const;

const EDIT_SET: ReadonlySet<string> = new Set([
  ...PLATE_EDIT_EVENTS,
  PROJECT_WIDE_EDIT_EVENT,
]);
const SAVED_SET: ReadonlySet<string> = new Set(SAVED_EVENTS);

/** Does this event name dirty the project? */
export function isEditEvent(name: string): boolean {
  return EDIT_SET.has(name);
}

/** Does this event name return the project to a clean baseline? */
export function isSavedEvent(name: string): boolean {
  return SAVED_SET.has(name);
}

interface EditPayload {
  data?: { plate_id?: number };
}

/** Subscribe to content edits. `onPlate(plateId)` fires for a plate-scoped
 *  edit; `onAll()` for a project-wide one. Returns a single unlisten that
 *  tears down every subscription. */
export async function listenPlateEdits(
  onPlate: (plateId: number) => void,
  onAll: () => void,
): Promise<UnlistenFn> {
  const uns: UnlistenFn[] = [];
  for (const name of PLATE_EDIT_EVENTS) {
    uns.push(
      await listen<EditPayload>(name, (e) => {
        const plateId = e.payload?.data?.plate_id;
        if (plateId != null) onPlate(plateId);
      }),
    );
  }
  uns.push(await listen(PROJECT_WIDE_EDIT_EVENT, () => onAll()));
  return () => uns.forEach((u) => u());
}
