// Tauri invoke wrappers for the autosave + recovery flow.

import { invoke } from "@tauri-apps/api/core";

/** One recoverable autosave file. Mirror of Rust's
 * `core::project::autosave::AutosaveEntry`. The recovery dialog
 * renders one row per entry. */
export interface AutosaveEntry {
  /** Filename stem == the project's uuid at the time of save. */
  uuid: string;
  /** Absolute path on disk; passed back to `project_load` when
   * the user picks Recover. */
  path: string;
  /** Unix seconds; the dialog renders this as a human-friendly
   * relative time. */
  modified_unix_secs: number;
  size_bytes: number;
}

/** Start the 30-second autosave worker. Idempotent — re-enabling
 * an already-running worker is a silent backend no-op. */
export function autosaveEnable(): Promise<void> {
  return invoke("project_autosave_enable");
}

/** Stop the autosave worker. Idempotent. */
export function autosaveDisable(): Promise<void> {
  return invoke("project_autosave_disable");
}

/** List recoverable autosave files (newest first). Returns an
 * empty array when no recovery candidates exist; the dialog
 * gates on that to decide whether to surface. */
export function autosaveList(): Promise<AutosaveEntry[]> {
  return invoke<AutosaveEntry[]>("project_autosave_list");
}

/** Delete one autosave file by uuid. Wires the dialog's
 * "Discard" button. Silent no-op when the file isn't present. */
export function autosaveDrop(uuid: string): Promise<void> {
  return invoke("project_autosave_drop", { uuid });
}

/** Load a saved/autosaved project file. Reuses the existing
 * `project_load` command — the recovery dialog hands it the
 * entry's `path` when the user picks Recover. */
export function projectLoad(path: string): Promise<void> {
  return invoke("project_load", { path });
}
