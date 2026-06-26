// Tauri invoke wrappers for the plate-tab strip.
//
// Mirrors the per-command `src/settings/overrideCommands.ts`
// pattern: thin, named wrappers so callers don't have to remember
// the command-name + arg-key strings, and tests can mock a single
// surface.

import { invoke } from "@tauri-apps/api/core";
import type { PlateId } from "../viewport/types";

/** Append a new plate to the project. Returns the freshly-allocated
 * `PlateId` so the caller can immediately switch focus or rename.
 * `printerIdentity === undefined` matches Rust's `None` (unbound
 * plate); the printer may be assigned later via the settings
 * printer-picker. */
export function addPlate(
  printerIdentity?: string | null,
): Promise<PlateId> {
  return invoke<PlateId>("scene_add_plate", {
    printerIdentity: printerIdentity ?? null,
  });
}

/** Remove a plate. Backend errors with `LastPlate` if it would
 * leave the project empty — the UI should hide the close button on
 * the only remaining tab to avoid that round-trip. */
export function removePlate(plateId: PlateId): Promise<void> {
  return invoke("scene_remove_plate", { plateId });
}

/** Switch the active plate. Silent backend no-op when already
 * active — caller doesn't need to dedupe. */
export function setActivePlate(plateId: PlateId): Promise<void> {
  return invoke("scene_set_active_plate", { plateId });
}

/** Rename a plate (dblclick → input → Enter/blur). Backend trims
 * the input and rejects empty / over-200-byte results — callers
 * should surface the rejection via the returned Promise's catch
 * (the tab strip falls back to the prior name on error). */
export function renamePlate(plateId: PlateId, name: string): Promise<void> {
  return invoke("scene_rename_plate", { plateId, name });
}
