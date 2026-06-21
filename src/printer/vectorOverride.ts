// Shared read/write helpers for per-index vector overrides (per-extruder
// settings and per-mode machine limits). `config_overrides` stores the
// whole serialized vector for a key; these read/write a single index,
// padding to the resolved length so a write never truncates other entries.

const splitVec = (s: string | undefined): string[] =>
  s == null || s === "" ? [] : s.split(",");

/** The resolved (base) vector for a key. */
export function resolvedVec(
  resolved: Record<string, string>,
  key: string,
): string[] {
  return splitVec(resolved[key]);
}

/** The current vector for a key — the override if set, else resolved. */
export function currentVec(
  overrides: Record<string, string>,
  resolved: Record<string, string>,
  key: string,
): string[] {
  return key in overrides ? splitVec(overrides[key]) : splitVec(resolved[key]);
}

/** Value at `index` of the current vector (null if absent). */
export function vecElem(
  overrides: Record<string, string>,
  resolved: Record<string, string>,
  key: string,
  index: number,
): string | null {
  return currentVec(overrides, resolved, key)[index] ?? null;
}

/** True when this index's value differs from the resolved base (the key
 *  may be overridden at a different index). */
export function elemOverridden(
  overrides: Record<string, string>,
  resolved: Record<string, string>,
  key: string,
  index: number,
): boolean {
  if (!(key in overrides)) return false;
  const cur = currentVec(overrides, resolved, key);
  const base = resolvedVec(resolved, key);
  return (cur[index] ?? "") !== (base[index] ?? "");
}

/** Set `next` at `index` and persist: clears the whole key once every
 *  entry matches resolved again, otherwise writes the full vector. */
export function setVecElem(
  overrides: Record<string, string>,
  resolved: Record<string, string>,
  key: string,
  index: number,
  next: string,
  onSet: (key: string, value: string) => void,
  onClear: (key: string) => void,
): void {
  const base = resolvedVec(resolved, key);
  const cur = currentVec(overrides, resolved, key);
  const len = Math.max(base.length, cur.length, index + 1);
  const vec = Array.from({ length: len }, (_, i) => cur[i] ?? base[i] ?? "");
  vec[index] = next;
  if (vec.join(",") === base.join(",")) onClear(key);
  else onSet(key, vec.join(","));
}
