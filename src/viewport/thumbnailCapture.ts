// Bridges the live viewport renderer to the send path: the ViewportCanvas
// registers a capture closure (it owns the rendered scene); the rest of the
// app calls `captureThumbnail()` without prop-drilling a ref through the tree.
// Module-level singleton — there's exactly one viewport at a time.
//
// The wrinkle: the edit viewport UNMOUNTS in G-code preview mode (App.tsx
// swaps it for PreviewWorkspace), and send/export happen *after* slicing,
// i.e. in preview. So we cache the last successful capture. `refreshCache`
// is called at slice start — while the edit scene is still mounted — and
// send/export read the cache when the live closure is gone.

// The capture may be sync (a renderer returns a canvas inline) or async (the
// wgpu viewport renders offscreen in Rust via IPC, then encodes the PNG).
export type ThumbnailCapture = (size?: number) => string | null | Promise<string | null>;

let live: ThumbnailCapture | null = null;
let cached: string | null = null;

/** The viewport registers its capture closure on mount, clears it (pass
 *  `null`) on unmount. */
export function registerThumbnailCapture(fn: ThumbnailCapture | null): void {
  live = fn;
}

/** Last cached capture, or a fresh *synchronous* one if the live viewport gives
 *  it inline. An async (wgpu) capture can't render here — it relies on
 *  `refreshThumbnailCache` having primed the cache at slice start. Returns base64
 *  PNG or `null`; never throws. */
export function captureThumbnail(size = 512): string | null {
  if (live) {
    try {
      const r = live(size);
      if (typeof r === "string") {
        cached = r;
        return r;
      }
    } catch {
      // fall through to the cache
    }
  }
  return cached;
}

/** Force a fresh capture and store it as the cache. Call (awaited) while the
 *  edit viewport is mounted (e.g. at slice start) so a thumbnail survives the
 *  switch to preview mode. Handles sync + async captures; keeps the prior cache
 *  if the render fails. */
export async function refreshThumbnailCache(size = 512): Promise<string | null> {
  if (live) {
    try {
      const png = await live(size);
      if (png) cached = png;
    } catch {
      // keep the prior cache
    }
  }
  return cached;
}
