// File open/save dialogs, backed by the in-process Rust commands
// (`dialog_open_file` / `dialog_save_file`).
//
// We don't use `@tauri-apps/plugin-dialog`'s open/save for files: on
// Linux those go through xdg-desktop-portal, which is unreliable in the
// flatpak sandbox. The Rust side drives GTK's FileChooserDialog directly
// (and rfd on macOS/Windows). Message dialogs still use the plugin —
// those are in-process everywhere.

import { invoke } from "@tauri-apps/api/core";

export interface DialogFilter {
  name: string;
  extensions: string[];
}

/** Pick an existing file to open. Resolves to the path, or null if cancelled. */
export function openFile(opts: {
  title?: string;
  filters: DialogFilter[];
}): Promise<string | null> {
  return invoke<string | null>("dialog_open_file", {
    title: opts.title ?? null,
    filters: opts.filters,
  });
}

/** Pick a destination to save to. Resolves to the path, or null if cancelled. */
export function saveFile(opts: {
  title?: string;
  defaultPath?: string;
  filters: DialogFilter[];
}): Promise<string | null> {
  return invoke<string | null>("dialog_save_file", {
    title: opts.title ?? null,
    defaultPath: opts.defaultPath ?? null,
    filters: opts.filters,
  });
}
