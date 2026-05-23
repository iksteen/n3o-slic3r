// Category grouping (PR-4-3) — FR-UI-1.
//
// `slicer_options` (Phase 1 / PR-4-1) returns options in libslic3r
// declaration order. The Settings UI groups them by `category` for
// navigation; within each group, declaration order carries through
// (which is libslic3r's choice — Simple-mode options first, then
// Advanced, then Expert tend to follow that order naturally).
//
// Categories with zero visible options after filtering are elided
// from the sidebar.

import type { OptionSummary, OptMode } from "../types";

/** libslic3r's declaration order for categories (first-mention
 *  walk through `external/OrcaSlicer/src/libslic3r/PrintConfig.cpp`).
 *  Categories not in this list fall to the end in alphabetic order.
 *  Update when the audit surfaces a new category, not when the FFI
 *  ships one — categories the FFI emits that aren't in this list
 *  are warning-logged and bucketed last so a libslic3r upstream
 *  change doesn't silently drop options. */
export const CATEGORY_ORDER = [
  "Quality",
  "Strength",
  "Layers and Perimeters",
  "Speed",
  "Support",
  "Extruders",
  "Flush options",
  "Machine limits",
  "Advanced",
  "Others",
  "Other",
] as const;

/** Single-letter glyph the `cat-rail-icon` shows next to each
 *  category. Pulled to roughly match the mockup's hand-picked set
 *  while extending to libslic3r's full category vocabulary.
 *  Unknown categories get `·` so the sidebar still renders. */
const CATEGORY_ICON: Record<string, string> = {
  Quality: "Q",
  Strength: "W",
  "Layers and Perimeters": "L",
  Speed: "S",
  Support: "T",
  Extruders: "E",
  "Flush options": "F",
  "Machine limits": "M",
  Advanced: "A",
  Others: "O",
  Other: "O",
};

export type CategoryGroup<O extends OptionSummary = OptionSummary> = {
  /** Stable category key matching libslic3r's `def->category` string. */
  id: string;
  /** Display name (same as `id` for now — translations land later). */
  name: string;
  /** Single-letter glyph for the rail. */
  icon: string;
  /** Visible options in declaration order. */
  settings: O[];
};

/** Apply the FR-UI-2 mode filter — Simple shows Simple-mode
 *  options only; Advanced includes Simple + Advanced; Expert
 *  includes everything except Develop (which stays dev-only). */
export type ModeFilter = "simple" | "advanced" | "expert" | "develop";

/** Return the mode "level" a filter passes — higher levels include
 *  lower ones. Develop is the highest (all-pass) tier. */
function modeLevel(mode: OptMode | ModeFilter): number {
  switch (mode) {
    case "simple":
      return 0;
    case "advanced":
      return 1;
    case "expert":
      return 2;
    case "develop":
      return 3;
  }
}

/** True iff an option's `mode` should be visible at the active
 *  filter level. The filter is "show this mode and everything
 *  simpler" — Advanced shows Simple+Advanced, etc. */
export function passesMode(opt: OptionSummary, filter: ModeFilter): boolean {
  return modeLevel(opt.mode) <= modeLevel(filter);
}

/** Group + order options. Returns the visible category list with
 *  empty categories elided. Within each category, the input order
 *  is preserved. Generic over the option type so callers can pass
 *  `PrinterAwareOptionSummary` and keep the `hidden` flag through
 *  the grouping pass. */
export function categorize<O extends OptionSummary>(
  options: readonly O[],
): CategoryGroup<O>[] {
  const byCategory = new Map<string, O[]>();
  for (const opt of options) {
    const key = opt.category ?? "Other";
    const bucket = byCategory.get(key);
    if (bucket) bucket.push(opt);
    else byCategory.set(key, [opt]);
  }

  // Walk the canonical order first, then any remaining categories
  // sorted alphabetically (so unexpected upstream additions are
  // visible without surprise reordering).
  const seen = new Set<string>();
  const out: CategoryGroup<O>[] = [];
  for (const id of CATEGORY_ORDER) {
    const settings = byCategory.get(id);
    if (!settings || settings.length === 0) continue;
    out.push({
      id,
      name: id,
      icon: CATEGORY_ICON[id] ?? "·",
      settings,
    });
    seen.add(id);
  }
  const trailing = [...byCategory.keys()]
    .filter((k) => !seen.has(k))
    .sort();
  for (const id of trailing) {
    const settings = byCategory.get(id) ?? [];
    if (settings.length === 0) continue;
    out.push({
      id,
      name: id,
      icon: CATEGORY_ICON[id] ?? "·",
      settings,
    });
  }
  return out;
}

/** Count summary returned by `categoryOverrideCounts` so the rail
 *  can render `overrides/total` next to each entry (PR-4-7 fills
 *  this from the cascade trace; PR-4-3 ships the shape + total). */
export type CategoryCounts = {
  total: number;
  overrides: number;
};

/** Pure helper: derive per-category total + override-count from
 *  the grouped list and a set of overridden keys. PR-4-7 populates
 *  the override set from the trace; for PR-4-3 alone, callers pass
 *  an empty set and only the `total` is meaningful. */
export function categoryCounts<O extends OptionSummary>(
  groups: readonly CategoryGroup<O>[],
  overriddenKeys: ReadonlySet<string>,
): Map<string, CategoryCounts> {
  const out = new Map<string, CategoryCounts>();
  for (const g of groups) {
    let overrides = 0;
    for (const s of g.settings) if (overriddenKeys.has(s.key)) overrides++;
    out.set(g.id, { total: g.settings.length, overrides });
  }
  return out;
}
