// Tauri invoke wrappers for the printer picker.
//
// Same pattern as `plateCommands.ts` / `overrideCommands.ts`:
// thin, named wrappers so callers don't have to repeat the
// command name + arg keys, and tests can mock one surface.

import { invoke } from "@tauri-apps/api/core";
import type { PlateId } from "../viewport/types";
import type { PrinterProfileJson } from "../settings/resolve";

/** Picker-facing entry. Mirror of Rust's
 * `core::printer::registry::CatalogEntry`. Carries the identity slug
 * + the full `PrinterProfile` — the picker chip + menu read the
 * summary fields (model + supported_build_plates) and the
 * settings panel host reads the full profile to feed
 * `cascade_resolve`. */
export interface PrinterCatalogEntry {
  identity: string;
  profile: PrinterProfileJson;
  experimental?: boolean;
}

/** Mirror of `PrinterChangeReport`. */
export interface PrinterChangeReport {
  plate_id: PlateId;
  previous_printer: string | null;
  new_printer: string;
  new_build_plate: string;
}

/** Fetch the bundled printer catalog. Static data — the picker
 * fetches once per app launch and caches the result. */
export function printerCatalog(): Promise<PrinterCatalogEntry[]> {
  return invoke<PrinterCatalogEntry[]>("printer_catalog");
}

/** Rebind a plate to a different `PrinterInstance` (by id). The
 *  bed currently loaded on the instance follows from the instance
 *  itself — change it via `setInstanceBed` (`printerInstance.ts`). */
export function rebindPlatePrinter(
  plateId: PlateId,
  instanceId: string,
): Promise<PrinterChangeReport> {
  return invoke<PrinterChangeReport>("scene_rebind_plate_printer", {
    plateId,
    instanceId,
  });
}

/** Clear a plate's printer binding. Used when the user deletes
 *  their last printer — there's no fallback to rebind to, so we
 *  null the binding before the workspace transitions to the
 *  add-printer empty state. Without this the stale UUID lingers
 *  on the plate and would route the next add's auto-bind only on
 *  the active plate (others would keep pointing at the deleted
 *  printer). */
export function unbindPlatePrinter(plateId: PlateId): Promise<void> {
  return invoke<void>("scene_unbind_plate_printer", { plateId });
}

