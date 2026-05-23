// Real `invoke()` wrappers for per-object cascade overrides
// (PR-5-7 frontend).
//
// PR-4-9 shipped the SettingsPanel with stub `onSetObjectOverride`
// / `onClearObjectOverride` props that the panel host (App.tsx,
// integrated by PR-5-9) is expected to wire to backend storage.
// PR-5-7 added the backend half (storage + Tauri commands on
// scene::commands); this module is what the host calls.
//
// Each wrapper takes a `plateId` + `objectId` because per-object
// overrides are scoped per-plate (the same object id on a
// different plate could carry different overrides — PR-5-11's
// move-between-plates relies on that). The host (PR-5-9) supplies
// these from the active plate + selected object.
//
// **Returns a Promise** — fire-and-forget on the SettingsPanel
// caller side is fine; the backend re-emits the panel's resolve
// loop via `scene:object_overrides_changed`, which re-runs
// `cascade_resolve` automatically.

import { invoke } from "@tauri-apps/api/core";
import type { ObjectId, PlateId } from "../viewport/types";

/** Upsert one cascade override key on a specific (plate, object).
 * Identical to a previous value is a silent backend no-op (no
 * event fires); the panel doesn't have to dedupe here. */
export function setObjectOverride(
  plateId: PlateId,
  objectId: ObjectId,
  key: string,
  value: string,
): Promise<void> {
  return invoke("scene_object_override_set", {
    plateId,
    objectId,
    key,
    value,
  });
}

/** Drop one cascade override key on a specific (plate, object).
 * Silent no-op when the key wasn't present — safe to call from
 * a per-row reset button without checking presence first. */
export function clearObjectOverride(
  plateId: PlateId,
  objectId: ObjectId,
  key: string,
): Promise<void> {
  return invoke("scene_object_override_clear", {
    plateId,
    objectId,
    key,
  });
}

/** Wipe every cascade override on a specific (plate, object).
 * Wires the "reset all object overrides" button in the Object
 * tab. Silent no-op when the object had no overrides. */
export function clearAllObjectOverrides(
  plateId: PlateId,
  objectId: ObjectId,
): Promise<void> {
  return invoke("scene_object_override_clear_all", {
    plateId,
    objectId,
  });
}

/** Build a `{ onSetObjectOverride, onClearObjectOverride }` pair
 * pre-bound to a specific (plate, object). Lets the SettingsPanel
 * host (PR-5-9) hand the panel ready-to-call callbacks without
 * the panel ever needing to know plate/object ids.
 *
 * Pass `null` for either id to get callbacks that no-op silently
 * — useful when the Object tab is reachable but no object is
 * selected. */
export function makeObjectOverrideCallbacks(
  plateId: PlateId | null,
  objectId: ObjectId | null,
): {
  onSetObjectOverride: (key: string, value: string) => void;
  onClearObjectOverride: (key: string) => void;
} {
  if (plateId === null || objectId === null) {
    return {
      onSetObjectOverride: () => {
        // No-op: no plate/object context.
      },
      onClearObjectOverride: () => {
        // No-op.
      },
    };
  }
  return {
    onSetObjectOverride: (key, value) => {
      void setObjectOverride(plateId, objectId, key, value).catch((err) => {
        console.error("[settings] setObjectOverride failed", err);
      });
    },
    onClearObjectOverride: (key) => {
      void clearObjectOverride(plateId, objectId, key).catch((err) => {
        console.error("[settings] clearObjectOverride failed", err);
      });
    },
  };
}
