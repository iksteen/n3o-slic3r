// Diff view helpers (PR-4-10) — Execution Plan §6 cut-candidate-
// but-shipping deliverable. Lets the user see at a glance which
// settings differ from a baseline.

import type { ResolvedMap } from "./resolve";

export type DiffMode = "all" | "from-default" | "from-save";

const STORAGE_KEY = "n3o.settings.diff_mode";

export function readStoredDiffMode(): DiffMode {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw === "all" || raw === "from-default" || raw === "from-save") return raw;
  } catch {
    // localStorage may be disabled — fall through.
  }
  return "all";
}

export function writeStoredDiffMode(mode: DiffMode): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, mode);
  } catch {
    // ignore quota / disabled
  }
}

/** Keys whose resolved value differs from the baseline. Pure /
 *  cheap — O(min(resolved, baseline)) string compares. */
export function computeDiff(
  resolved: ResolvedMap,
  baseline: ResolvedMap,
): Set<string> {
  const out = new Set<string>();
  // Walk the union of keys so additions on either side are
  // captured. Most rows are in both.
  for (const key of Object.keys(resolved)) {
    if (resolved[key]?.value !== baseline[key]?.value) out.add(key);
  }
  for (const key of Object.keys(baseline)) {
    if (!(key in resolved)) out.add(key);
  }
  return out;
}

/** True when the diff mode passes this key. `"all"` always
 *  passes; `"from-default"` and `"from-save"` pass only keys in
 *  the diff set. */
export function passesDiff(
  key: string,
  mode: DiffMode,
  diff: ReadonlySet<string>,
): boolean {
  if (mode === "all") return true;
  return diff.has(key);
}
