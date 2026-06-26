// Lets the axis legend snap the live viewport camera to an axis-aligned view
// without prop-drilling a ref through App. Module-level singleton — the prepare
// and preview viewports (both wgpu) never mount at once, so whichever is
// live registers its handler; same pattern as thumbnailCapture.

export type AxisView = "x" | "y" | "z";

let live: ((axis: AxisView) => void) | null = null;

/** The viewport registers its axis-snap handler on mount, clears it (pass
 *  `null`) on unmount. */
export function registerAxisView(fn: ((axis: AxisView) => void) | null): void {
  live = fn;
}

/** Snap the camera to look along `axis`. No-op when the viewport is unmounted. */
export function setAxisView(axis: AxisView): void {
  live?.(axis);
}
