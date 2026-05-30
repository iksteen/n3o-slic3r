// Compact duration formatting, shared across the preview stats and the
// Devices monitor so the convention doesn't drift between them.

/** Format a duration in seconds as a compact `Xh YYm` / `Ym YYs` /
 *  `Ys` string (minutes/seconds zero-padded). Pass `zero` to return a
 *  sentinel (e.g. `"—"`) for non-positive durations; without it, zero
 *  and negatives clamp to `"0s"`. */
export function formatDuration(seconds: number, zero?: string): string {
  if (zero != null && seconds <= 0) return zero;
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}h ${String(m).padStart(2, "0")}m`;
  if (m > 0) return `${m}m ${String(sec).padStart(2, "0")}s`;
  return `${sec}s`;
}
