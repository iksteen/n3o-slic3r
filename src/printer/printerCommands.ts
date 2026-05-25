// Tauri invoke wrappers for the printer picker (PR-5-4).
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
 * summary fields (model/slot_count/supported_build_plates) and the
 * settings panel host reads the full profile to feed
 * `cascade_resolve`. */
export interface PrinterCatalogEntry {
  identity: string;
  profile: PrinterProfileJson;
}

/** Mirror of `PrinterChangeReport`. `incompatible` + `clamped` are
 * always empty until the validation walk lands; surfaced anyway so
 * the picker can render the warning panel once they populate. */
export interface PrinterChangeReport {
  plate_id: PlateId;
  previous_printer: string | null;
  new_printer: string;
  new_build_plate: string;
  incompatible: IncompatibleSetting[];
  clamped: ClampedSetting[];
}
export interface IncompatibleSetting {
  key: string;
  value: string;
  reason: string;
}
export interface ClampedSetting {
  key: string;
  from: string;
  to: string;
}

/** Fetch the bundled printer catalog. Static data — the picker
 * fetches once per app launch and caches the result. */
export function printerCatalog(): Promise<PrinterCatalogEntry[]> {
  return invoke<PrinterCatalogEntry[]>("printer_catalog");
}

/** Rebind a plate to a different printer by identity. The bed
 * currently loaded on the bound `PrinterInstance` follows from the
 * instance itself — change it via `printerInstanceSetBed`. */
export function rebindPlatePrinter(
  plateId: PlateId,
  printerIdentity: string,
): Promise<PrinterChangeReport> {
  return invoke<PrinterChangeReport>("scene_rebind_plate_printer", {
    plateId,
    printerIdentity,
  });
}

/** Install a fully-resolved `PrinterProfile` on the active plate
 * (Phase 2 bootstrap path, kept for App.tsx's first-mount default).
 * The picker flow uses `rebindPlatePrinter` instead. */
export function setActivePrinter(
  printer: PrinterProfileJson | null,
): Promise<void> {
  return invoke("scene_set_active_printer", { printer });
}

/** Change the bed currently loaded on a `PrinterInstance`. The
 * backend validates the identity against the instance's bound
 * printer profile's `supported_build_plates` and emits
 * `printer:instance_changed`. */
export function printerInstanceSetBed(
  id: string,
  bedIdentity: string,
): Promise<void> {
  return invoke("printer_instance_set_bed", { id, bedIdentity });
}
