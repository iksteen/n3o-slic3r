// Project file commands for the File menu (Open / Save / Save as).
//
// `project_save` writes to a path without changing the project's source
// path; `project_save_as` writes and adopts the path as the new source
// (so subsequent "Save project" goes there). `projectLoad` is reused
// from the autosave-recovery wrapper — it's the same `project_load`
// command.
import { invoke } from "@tauri-apps/api/core";

/** Save the project to `path`, leaving its source path unchanged. */
export function projectSave(path: string): Promise<void> {
  return invoke("project_save", { path });
}

/** Save the project to `path` and adopt it as the project's source path. */
export function projectSaveAs(path: string): Promise<void> {
  return invoke("project_save_as", { path });
}

export { projectLoad } from "./autosaveCommands";
