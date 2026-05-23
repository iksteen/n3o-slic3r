// Project-tier override invoke wrappers (PR-5-9).
//
// Mirrors `overrideCommands.ts` one tier up: project overrides
// apply to a whole plate (and through the cascade to every object
// on it). Backend: `scene_project_override_set/clear/clear_all`.
//
// The SettingsPanel's Project tab calls `setProjectOverride`
// whenever a row is edited; the panel host (App.tsx) binds the
// callback to the active plate via `makeProjectOverrideCallbacks`.

import { invoke } from "@tauri-apps/api/core";
import type { PlateId } from "../viewport/types";

/** Upsert one project-tier override on a plate. Identical-value
 * writes are a silent backend no-op (no event); the panel doesn't
 * dedupe here. */
export function setProjectOverride(
  plateId: PlateId,
  key: string,
  value: string,
): Promise<void> {
  return invoke("scene_project_override_set", { plateId, key, value });
}

/** Drop one project-tier override key from a plate. Silent no-op
 * when the key wasn't present. */
export function clearProjectOverride(
  plateId: PlateId,
  key: string,
): Promise<void> {
  return invoke("scene_project_override_clear", { plateId, key });
}

/** Wipe every project-tier override on a plate. */
export function clearAllProjectOverrides(plateId: PlateId): Promise<void> {
  return invoke("scene_project_override_clear_all", { plateId });
}

/** Build `{ onSetProjectOverride, onClearProjectOverride }` pre-bound
 * to a specific plate. `null` plateId returns silent no-ops so the
 * panel can mount before a plate is active. */
export function makeProjectOverrideCallbacks(plateId: PlateId | null): {
  onSetProjectOverride: (key: string, value: string) => void;
  onClearProjectOverride: (key: string) => void;
} {
  if (plateId === null) {
    return {
      onSetProjectOverride: () => undefined,
      onClearProjectOverride: () => undefined,
    };
  }
  return {
    onSetProjectOverride: (key, value) => {
      void setProjectOverride(plateId, key, value).catch((err) => {
        console.error("[settings] setProjectOverride failed", err);
      });
    },
    onClearProjectOverride: (key) => {
      void clearProjectOverride(plateId, key).catch((err) => {
        console.error("[settings] clearProjectOverride failed", err);
      });
    },
  };
}
