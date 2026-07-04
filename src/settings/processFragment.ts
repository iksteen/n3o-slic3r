// Mirror of `core::profile_library::ProcessFragmentSummary` and the
// Tauri wrappers for the Quality picker.
//
// Backend reads each process fragment's `[meta] available_for` at
// startup; this command filters to processes whose listing includes
// the active (printer, nozzle) pair.

import { invoke } from "@tauri-apps/api/core";
import type { PlateId } from "../viewport/types";

export interface ProcessAvailability {
  printer: string;
  nozzle: string;
}

export interface ProcessFragmentSummary {
  /** Wire identity — what the picker writes back via
   *  `setPlateQualityProfile`. Matches the on-disk filename
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
  /** True when the user has stamped overrides onto this profile. The
   *  picker bolds it and offers Revert. */
  edited?: boolean;
  /** True for a named custom profile (a "save as…" clone). The picker
   *  offers Delete (instead of Revert) for these. */
  custom?: boolean;
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

/** Set a plate's process/quality profile (a bundled process slug),
 *  overriding the bound instance's default for that plate only. Emits
 *  `PlateChanged`, which the session listens on to refetch the
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

/** A stamped user override profile over one bundled process, scoped to a
 *  printer. Mirror of `core::process::UserProcess`. */
export interface UserProcess {
  id: string;
  printer: string;
  base: string;
  /** Display name for a named custom clone; null for a stamp-in-place edit. */
  name: string | null;
  overrides: Record<string, string>;
}

/** The stamped override profile for a printer's process, or `null` if
 *  pristine. Drives the picker's bold name + Revert affordance. */
export async function getUserProcess(
  printer: string,
  base: string,
): Promise<UserProcess | null> {
  return invoke<UserProcess | null>("user_process_get", { printer, base });
}

/** Per-plate placement keys the viewport drag writes into project overrides
 *  (the wipe/prime-tower position). Stamping excludes them — a dragged tower
 *  must never bake into the shared quality profile. Mirrors the backend
 *  `STAMP_EXCLUDED_KEYS` in `core::project::commands`. */
export const STAMP_EXCLUDED_KEYS: readonly string[] = [
  "wipe_tower_x",
  "wipe_tower_y",
];

/** Stamp the active plate's current quality edits (its Process-bucket
 *  project-tier overrides, minus placement keys) onto the selected profile
 *  as a per-user diff. With `clear`, the edits are then removed from the
 *  plate too (save then clear); otherwise they stay on the plate as well.
 *  No-op when there's nothing to save. */
export async function stampUserProcess(
  plateId: PlateId,
  clear: boolean,
): Promise<void> {
  await invoke("user_process_stamp", { plateId, clear });
}

/** Discard the plate's selected stamp-in-place profile's overrides — back to
 *  pristine bundled. With `apply`, the profile's settings are first written
 *  onto the plate's project tier (kept as project overrides) instead of being
 *  lost. */
export async function revertUserProcess(
  plateId: PlateId,
  apply: boolean,
): Promise<void> {
  await invoke("user_process_revert", { plateId, apply });
}

/** Save the plate's current quality settings as a new named custom profile
 *  (inheriting the selected profile's base + overrides) and switch the plate
 *  onto it. With `clear`, the merged edits are also removed from the plate.
 *  Returns the new profile's id. */
export async function duplicateUserProcess(
  plateId: PlateId,
  name: string,
  clear: boolean,
): Promise<string> {
  return invoke<string>("user_process_duplicate", { plateId, name, clear });
}

/** Delete the plate's selected named custom profile and switch the plate back
 *  to its default profile. With `apply`, the custom profile's settings are
 *  first written onto the plate's project tier (kept as project overrides)
 *  instead of being lost. No-op unless a custom profile is selected. */
export async function deleteUserProcess(
  plateId: PlateId,
  apply: boolean,
): Promise<void> {
  await invoke("user_process_delete", { plateId, apply });
}
