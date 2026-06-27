// Pure helpers + shared types for the cascade-resolved SettingsPanel.
// Extracted so the panel file stays the orchestrator and these stay
// independently testable.

import { passesMode, type ModeFilter } from "./nav";
import type {
  OptionSummary,
  PrinterAwareOptionSummary,
} from "./types";
import { defaultScalarFor } from "./types";
import type { ResolvedMap } from "./resolve";
import { winningLayerFor, type CascadeLayer } from "./layers";

/** Active editing-context tab for the panel + its rows. */
export type ContextLayer = "project" | "object";

/** The ladder's winner ✓: an override tier when one wins, else the
 *  cascade layer the resolved value was attributed to (so the ✓ lands on
 *  e.g. "Profile" when the process fragment is the winner). Distinct from
 *  `winningLayerFor`, which stays "cascade" for the row-tint logic that
 *  must not treat a fragment win as user-authored. */
export function ladderWinningLayer(
  key: string,
  resolved: ResolvedMap,
  projectOverrides: Record<string, string>,
  objectOverrides: Record<string, string>,
): CascadeLayer {
  const base = winningLayerFor(key, projectOverrides, objectOverrides);
  if (base !== "cascade") return base;
  return (resolved[key]?.source_layer as CascadeLayer | undefined) ?? "cascade";
}

/** Build the per-layer value snapshot the CascadeLadder reads.
 *  `default` is the engine schema default. The cascade-resolved value
 *  (`plate_cascade_resolve`) is placed under the layer it won from —
 *  its `source_layer` — so e.g. a process-fragment value shows under
 *  "Profile" (the `user` row) and a bed-fragment value under "Build
 *  plate". The override tiers (`project` / `object`) come straight from
 *  the panel's own override maps. Cascade rows with no contribution
 *  render as em-dash. */
export function buildLadderLayers(
  schema: OptionSummary,
  resolved: ResolvedMap,
  projectOverrides: Record<string, string>,
  objectOverrides: Record<string, string>,
): Map<CascadeLayer, string | null> {
  const map = new Map<CascadeLayer, string | null>();
  // `default` is the engine schema default — the genuine bottom of the
  // ladder, distinct from what our fragments resolve to.
  map.set("default", defaultScalarFor(schema));
  map.set("printer", null);
  map.set("build_plate", null);
  map.set("nozzle", null);
  map.set("filament", null);
  map.set("user", null);
  // Place the cascade-resolved value under the cascade layer it won
  // from. `source_layer` is a CascadeLayer id from the backend
  // (process → "user"/Profile, nozzle → "nozzle", bed → "build_plate",
  // filament → "filament", machine/topology → "printer"); fall back to
  // "printer" so a value is never silently dropped.
  const entry = resolved[schema.key];
  if (entry != null) {
    const layer = (entry.source_layer ?? "printer") as CascadeLayer;
    if (map.has(layer)) map.set(layer, entry.value);
  }
  map.set("project", projectOverrides[schema.key] ?? null);
  map.set("object", objectOverrides[schema.key] ?? null);
  return map;
}

/** Pure filter function used by the panel and exposed for vitest.
 *  `modified` (overridden at the active layer) bypasses the mode tier so a
 *  changed setting is never hidden by Simple/Advanced. */
export function filterRow(
  opt: PrinterAwareOptionSummary,
  mode: ModeFilter,
  search: string,
  modified = false,
): boolean {
  if (opt.hidden) {
    // Match the mockup behavior: when search is active, hidden
    // options are shown with a "not applicable" badge. This
    // filter excludes them in the no-search default view.
    if (search.trim() === "") return false;
  }
  if (!passesMode(opt, mode) && !modified) return false;
  if (search.trim() === "") return true;
  const needle = search.toLowerCase();
  return (
    opt.key.toLowerCase().includes(needle) ||
    (opt.label?.toLowerCase().includes(needle) ?? false) ||
    (opt.category?.toLowerCase().includes(needle) ?? false)
  );
}
