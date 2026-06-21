// User overrides for bundled filaments — invoke wrappers. A bundled
// filament is edited in place (it keeps its identity/name); the override
// profile is keyed by the bundled slug, created transparently on the first
// edit and removed when its last override is cleared. Every mutation emits
// `filament:changed` backend-side, invalidating the `filament_catalog`
// query so the picker's Revert affordance + the open editor refetch.

import { invoke } from "@tauri-apps/api/core";

/** Frontend mirror of `core::filament::UserFilament`. */
export interface UserFilament {
  /** Bundled fragment slug this overrides (and is identified by). */
  base: string;
  /** Filament-bucket scalar overrides, key → serialized value. */
  overrides: Record<string, string>;
}

/** The override profile for a bundled slug, or `null` if pristine. */
export async function getUserFilament(
  base: string,
): Promise<UserFilament | null> {
  return invoke<UserFilament | null>("user_filament_get", { base });
}

/** Discard all of a filament's user overrides — back to pristine bundled. */
export async function revertUserFilament(base: string): Promise<void> {
  await invoke("user_filament_revert", { base });
}

/** Set (or clear, with `value = null`) one filament-bucket override. Creates
 *  the override profile on first edit; clearing the last override reverts to
 *  pristine. Returns the resulting profile (overrides may be empty). */
export async function setFilamentOverride(
  base: string,
  key: string,
  value: string | null,
): Promise<UserFilament> {
  return invoke<UserFilament>("user_filament_set_override", {
    base,
    key,
    value,
  });
}

/** The filament's base (pre-override) scalar values — shown beneath any
 *  override in the editor. */
export async function resolvedFilamentConfig(
  base: string,
): Promise<Record<string, string>> {
  return invoke<Record<string, string>>("user_filament_resolved_config", {
    base,
  });
}
