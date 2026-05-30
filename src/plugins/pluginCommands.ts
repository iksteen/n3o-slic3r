// Plugin-management invoke wrappers + wire types.
//
// Mirrors `settings/projectOverrideCommands.ts`: thin `invoke`
// wrappers, camelCase args auto-mapped to the Rust commands'
// snake_case params, fire-and-forget with `.catch(console.error)`
// for the mutation calls.
//
// THE THREE LEVELS, THREE SOURCES
//   global  → backend plugin store    (plugin_set_global_*)
//   project → Project.user_overrides  (scene_user_override_*)
//   plate   → Plate.project_overrides (scene_project_override_*)
//
// Project + plate enablement / settings are encoded as override
// keys: `plugin.<name>.enabled` ("true"|"false") and
// `plugin.<name>.<settingKey>` (the setting's string form). Clearing
// a key means "inherit from the level above".

import { invoke } from "@tauri-apps/api/core";
import type { PlateId } from "../viewport/types";

/** One configurable setting a plugin exposes (manifest-declared). */
export interface SettingSummary {
  key: string;
  kind: "string" | "number" | "bool" | "enum";
  label: string | null;
  /** Default value, serialized as a string regardless of `kind`. */
  default: string;
  /** Allowed values for `enum` kind (the option list); empty
   *  otherwise. */
  values: string[];
}

/** A plugin as reported by `plugin_list`. The lean, real backend
 *  shape — no category / glyph / author / summary fields. */
export interface PluginSummary {
  name: string;
  version: string;
  /** Compose / post-process hooks the plugin registers. */
  hooks: string[];
  /** Printer models the plugin is scoped to, or `null` for "any". */
  printers: string[] | null;
  /** Cascade levels the plugin is available at
   *  ("global" | "project" | "plate"). */
  scopes: string[];
  /** Health: did the plugin load + pass its self-check. */
  enabled: boolean;
  /** Global-level enablement (the cascade root's resolved value). */
  globally_enabled: boolean;
  settings: SettingSummary[];
  /** Resolved global-level setting values, keyed by setting key.
   *  Absent keys fall back to the setting's `default`. */
  global_settings: Record<string, string>;
  /** Last load / self-check error, or `null` when healthy. */
  last_error: string | null;
}

/** Fetch the full plugin list. */
export function listPlugins(): Promise<PluginSummary[]> {
  return invoke<PluginSummary[]>("plugin_list");
}

// ── Global level ──────────────────────────────────────────────────

/** Enable / disable a plugin at the global (machine) level. */
export function setGlobalEnabled(name: string, enabled: boolean): void {
  void invoke("plugin_set_global_enabled", { name, enabled }).catch(
    console.error,
  );
}

/** Set one global-level setting. `value` is the JS scalar typed per
 *  the setting's `kind` (string / number / boolean). */
export function setGlobalSetting(
  name: string,
  key: string,
  value: string | number | boolean,
): void {
  void invoke("plugin_set_global_setting", { name, key, value }).catch(
    console.error,
  );
}

/** Reload a plugin from disk (re-runs its load + self-check). */
export function reloadPlugin(name: string): void {
  void invoke("plugin_reload", { name }).catch(console.error);
}

// ── Project level (Project.user_overrides) ────────────────────────

/** Upsert one project-tier override key. */
export function setUserOverride(key: string, value: string): void {
  void invoke("scene_user_override_set", { key, value }).catch(console.error);
}

/** Drop one project-tier override key (→ inherit from global). */
export function clearUserOverride(key: string): void {
  void invoke("scene_user_override_clear", { key }).catch(console.error);
}

// ── Plate level (Plate.project_overrides) ─────────────────────────

/** Upsert one plate-tier override key on a plate. */
export function setProjectOverride(
  plateId: PlateId,
  key: string,
  value: string,
): void {
  void invoke("scene_project_override_set", { plateId, key, value }).catch(
    console.error,
  );
}

/** Drop one plate-tier override key (→ inherit from project). */
export function clearProjectOverride(plateId: PlateId, key: string): void {
  void invoke("scene_project_override_clear", { plateId, key }).catch(
    console.error,
  );
}
