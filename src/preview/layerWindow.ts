// Pure LayerWindow transitions (PR-6-9).
//
// Mode switches preserve the user's current "visible layer"
// intent so the renderer doesn't snap when the user toggles
// between single / up-to / range. Keyboard step + clamp logic
// also lives here so the React component stays thin + the
// transitions are unit-testable.

import type { LayerWindow } from "./types";

/** Switch `current` into `targetMode`, preserving the visible
 * layer where the mapping is intuitive:
 *
 * - any → single: pick the current `max` (up-to / range) or
 *   current layer (already single).
 * - any → up-to: use the current `max` (up-to / range) or
 *   `layer` (single).
 * - any → range: keep `max` from current; pick `min = 0` when
 *   moving from single/up-to (the natural "start at the
 *   beginning" interpretation).
 */
export function switchMode(
  current: LayerWindow,
  targetMode: LayerWindow["mode"],
): LayerWindow {
  switch (targetMode) {
    case "single":
      return { mode: "single", layer: currentVisibleMax(current) };
    case "up-to":
      return { mode: "up-to", max: currentVisibleMax(current) };
    case "range":
      return {
        mode: "range",
        min: current.mode === "range" ? current.min : 0,
        max: currentVisibleMax(current),
      };
  }
}

/** Step the layer by `delta` (positive = forward, negative =
 * backward), clamped to `[0, layerCount-1]`. Behavior per mode:
 *
 * - single → step `layer`.
 * - up-to → step `max`.
 * - range → step `max` (the "top" of the visible band).
 *
 * `layerCount` is the total layer count (1-based); a 0-count
 * input is a no-op since there's nothing to step.
 */
export function stepLayer(
  current: LayerWindow,
  delta: number,
  layerCount: number,
): LayerWindow {
  if (layerCount <= 0) return current;
  const last = layerCount - 1;
  switch (current.mode) {
    case "single":
      return { mode: "single", layer: clamp(current.layer + delta, 0, last) };
    case "up-to":
      return { mode: "up-to", max: clamp(current.max + delta, 0, last) };
    case "range":
      return {
        mode: "range",
        min: current.min,
        max: clamp(current.max + delta, current.min, last),
      };
  }
}

/** Jump to the first or last layer per `target`. */
export function jumpTo(
  current: LayerWindow,
  target: "first" | "last",
  layerCount: number,
): LayerWindow {
  if (layerCount <= 0) return current;
  const last = layerCount - 1;
  const dest = target === "first" ? 0 : last;
  switch (current.mode) {
    case "single":
      return { mode: "single", layer: dest };
    case "up-to":
      return { mode: "up-to", max: dest };
    case "range":
      return {
        mode: "range",
        min: target === "first" ? 0 : Math.min(current.min, dest),
        max: dest,
      };
  }
}

/** Default window for a freshly-loaded preview: show only the
 * topmost layer, single-layer mode. Matches the Bambu Studio /
 * Orca / Prusa default. Showing every layer at once on a tall
 * print is visually unreadable — the user scrubs down from the
 * top via the slider or arrow keys to inspect specific layers. */
export function defaultWindow(layerCount: number): LayerWindow {
  return { mode: "single", layer: Math.max(0, layerCount - 1) };
}

/** Best-effort "what layer is currently visible at the top of
 * the window". Used by mode switches to preserve intent. */
function currentVisibleMax(window: LayerWindow): number {
  switch (window.mode) {
    case "single":
      return window.layer;
    case "up-to":
      return window.max;
    case "range":
      return window.max;
  }
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}
