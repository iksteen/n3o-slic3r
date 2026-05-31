// Mirror of `core::profile_library::ProcessFragmentSummary` and the
// Tauri wrappers for the Quality picker.
//
// Backend reads each process fragment's `[meta] available_for` at
// startup; this command filters to processes whose listing includes
// the active (printer, nozzle) pair.

import { invoke } from "@tauri-apps/api/core";
import type { PrinterInstance } from "../printer/printerInstance";
import type { PlateId } from "../viewport/types";

export interface ProcessAvailability {
  printer: string;
  nozzle: string;
}

export interface ProcessFragmentSummary {
  /** Wire identity — what the picker writes back via
   *  `setInstanceQualityProfile`. Matches the on-disk filename
   *  (e.g. `"0.20mm-standard"`). */
  slug: string;
  /** Picker-visible label, from `print_settings_id`
   *  (e.g. `"0.20mm Standard"`). */
  display_name: string;
  /** From the fragment's baseline `layer_height` field — feeds the
   *  chip's "0.20 mm" sub-line. `null` when the fragment didn't
   *  declare one (rare; baseline is normally always present). */
  layer_height_mm: number | null;
  /** Full set of (printer, nozzle) combos the fragment supports. The
   *  picker only surfaces fragments whose `available_for` matches
   *  the active pair, but the full list lets a future UI surface
   *  "also fits …" hints. */
  available_for: ProcessAvailability[];
}

/** Enumerate process fragments available for the active
 *  (printer, installed-nozzle-set). `printerFragmentSlug` is the
 *  printer directory slug (e.g. `"bambu-lab-a1-mini"`);
 *  `printerModel` is the human printer name from `machine.toml`
 *  (e.g. `"Bambu Lab A1 mini"`) — the metadata's `available_for`
 *  rows key off the latter. `installedNozzleDiameters` is the
 *  unique set of nozzle diameters installed across all of the
 *  printer's extruders (single-extruder printer: `["0.4"]`;
 *  mixed-nozzle U1 toolchanger: `["0.4", "0.6"]`). The backend
 *  surfaces a process when any nozzle in its `available_for` entry
 *  (split on `+` for composite specs like `"0.4+0.6"`) is in the
 *  installed set. */
export async function listProcessFragments(
  printerFragmentSlug: string,
  printerModel: string,
  installedNozzleDiameters: readonly string[],
): Promise<ProcessFragmentSummary[]> {
  return invoke<ProcessFragmentSummary[]>("process_fragment_list", {
    printerFragmentSlug,
    printerModel,
    installedNozzleDiameters,
  });
}

/** Update the instance's selected process fragment. Emits the same
 *  `printer:instance_changed` event the bed and nozzle setters use,
 *  so the cascade preview re-resolves. */
export async function setInstanceQualityProfile(
  id: string,
  qualityProfile: string,
): Promise<PrinterInstance> {
  return invoke<PrinterInstance>(
    "printer_instance_set_quality_profile",
    { id, qualityProfile },
  );
}

/** Set a plate's process/quality profile (a bundled process slug),
 *  overriding the bound instance's default for that plate only. Emits
 *  `PlateMetadataChanged`, which the session listens on to refetch the
 *  snapshot, so the picker + cascade ladder re-resolve. Pass `null` to
 *  clear the override and inherit the instance's profile again. */
export async function setPlateQualityProfile(
  plateId: PlateId,
  qualityProfile: string | null,
): Promise<void> {
  return invoke("project_set_plate_quality_profile", {
    plateId,
    qualityProfile,
  });
}
