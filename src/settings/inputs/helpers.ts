// Pure parse / format / validate helpers for the form components.
// Extracted from the component bodies so they can be
// tested without a DOM or React renderer — the project's existing
// vitest pattern is pure-logic-only (see src/slice/reducer.ts +
// __test__/reducer.test.ts), and the components stay thin wrappers
// over these helpers.

/** Numeric bounds + step pulled from `OptionSummary` for clamping
 *  and per-keystroke nudging. All optional — undefined = no
 *  constraint. The FFI's `min` / `max` defaults are `f64::MAX` /
 *  `-f64::MAX` for "unset", which we don't surface as constraints. */
export type NumericBounds = {
  min?: number;
  max?: number;
  step?: number;
};

/** Result of attempting to commit a typed value to a row. */
export type CommitResult<T> =
  | { ok: true; value: T; serialized: string }
  | { ok: false; error: string };

/** Parse a number from the user's free-text input. Accepts integer
 *  + decimal notation; rejects empty strings, non-numerics, NaN,
 *  and infinities. */
export function parseNumber(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const n = Number(trimmed);
  if (!Number.isFinite(n)) return null;
  return n;
}

/** Clamp to `[min, max]` and snap to `step` if both provided.
 *  Returns the input unchanged when no bound applies. */
export function clamp(value: number, bounds: NumericBounds): number {
  let out = value;
  if (bounds.min != null && out < bounds.min) out = bounds.min;
  if (bounds.max != null && out > bounds.max) out = bounds.max;
  return out;
}

/** Format a number for display, dropping trailing-zero noise
 *  (`1.20` → `1.2`) so the input doesn't visually churn on commit. */
export function formatNumber(value: number, decimals = 3): string {
  if (!Number.isFinite(value)) return "";
  return (Math.round(value * 10 ** decimals) / 10 ** decimals).toString();
}

/** Commit a number with bounds + serialization for the wire (libslic3r
 *  always parses string-serialized numbers, so we round-trip through
 *  string). */
export function commitNumber(
  text: string,
  bounds: NumericBounds,
): CommitResult<number> {
  const n = parseNumber(text);
  if (n == null) return { ok: false, error: "expected a number" };
  const clamped = clamp(n, bounds);
  return { ok: true, value: clamped, serialized: formatNumber(clamped) };
}

/** Parse a percent value. Accepts `"75"` or `"75%"` (libslic3r's
 *  `Percent` type serializes without the suffix, but the user may
 *  paste either form). */
export function parsePercent(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const stripped = trimmed.endsWith("%") ? trimmed.slice(0, -1) : trimmed;
  return parseNumber(stripped);
}

export function commitPercent(
  text: string,
  bounds: NumericBounds,
): CommitResult<number> {
  const n = parsePercent(text);
  if (n == null) return { ok: false, error: "expected a percent (0-100)" };
  const clamped = clamp(n, bounds);
  return { ok: true, value: clamped, serialized: formatNumber(clamped) };
}

/** FloatOrPercent: libslic3r stores both numeric value and a bool
 *  for `percent`. Serialization on the wire is `"42"` (float) or
 *  `"42%"` (percent). */
export type FloatOrPercent = { value: number; percent: boolean };

export function parseFloatOrPercent(text: string): FloatOrPercent | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const percent = trimmed.endsWith("%");
  const n = parseNumber(percent ? trimmed.slice(0, -1) : trimmed);
  if (n == null) return null;
  return { value: n, percent };
}

export function formatFloatOrPercent(v: FloatOrPercent): string {
  return v.percent ? `${formatNumber(v.value)}%` : formatNumber(v.value);
}

export function commitFloatOrPercent(
  text: string,
  bounds: NumericBounds,
): CommitResult<FloatOrPercent> {
  const parsed = parseFloatOrPercent(text);
  if (parsed == null) return { ok: false, error: "expected a number or N%" };
  const clamped = { ...parsed, value: clamp(parsed.value, bounds) };
  return { ok: true, value: clamped, serialized: formatFloatOrPercent(clamped) };
}

/** Validate a `#RRGGBB` hex color string. Accepts uppercase or
 *  lowercase digits; rejects `#RGB` shorthand (libslic3r expects
 *  the full 6-digit form) and any non-hex character. */
export function isValidHexColor(text: string): boolean {
  return /^#[0-9a-fA-F]{6}$/.test(text.trim());
}

export function commitColor(text: string): CommitResult<string> {
  const t = text.trim();
  if (!isValidHexColor(t)) return { ok: false, error: "expected #RRGGBB hex" };
  return { ok: true, value: t.toLowerCase(), serialized: t.toLowerCase() };
}

/** Parse `"1"` / `"true"` / `"0"` / `"false"` to bool. libslic3r
 *  serializes bools as `"1"` / `"0"`. */
export function parseBool(text: string): boolean | null {
  const t = text.trim().toLowerCase();
  if (t === "1" || t === "true") return true;
  if (t === "0" || t === "false") return false;
  return null;
}

export function formatBool(value: boolean): string {
  return value ? "1" : "0";
}
