// Bridges the live viewport renderer to the send path: the ViewportCanvas
// registers a capture closure (it owns the Three.js scene); the rest of the
// app calls `captureThumbnail()` without prop-drilling a ref through the tree.
// Module-level singleton — there's exactly one viewport at a time.
//
// The wrinkle: the edit viewport UNMOUNTS in G-code preview mode (App.tsx
// swaps it for PreviewWorkspace), and send/export happen *after* slicing,
// i.e. in preview. So we cache the last successful capture. `refreshCache`
// is called at slice start — while the edit scene is still mounted — and
// send/export read the cache when the live closure is gone.

export type ThumbnailCapture = (size?: number) => string | null;

let live: ThumbnailCapture | null = null;
let cached: string | null = null;

/** ViewportCanvas registers its capture closure on mount, clears it (pass
 *  `null`) on unmount. */
export function registerThumbnailCapture(fn: ThumbnailCapture | null): void {
  live = fn;
}

/** Render fresh from the live viewport if mounted (and update the cache),
 *  else fall back to the last cached capture. Returns base64 PNG or `null`.
 *  Never throws — a failed capture just ships the print without a thumbnail. */
export function captureThumbnail(size = 512): string | null {
  const fresh = renderLive(size);
  return fresh ?? cached;
}

/** Force a fresh capture and store it as the cache. Call while the edit
 *  viewport is mounted (e.g. at slice start) so a thumbnail survives the
 *  switch to preview mode. Keeps the prior cache if the render fails. */
export function refreshThumbnailCache(size = 512): string | null {
  renderLive(size);
  return cached;
}

function renderLive(size: number): string | null {
  if (!live) return null;
  try {
    const png = live(size);
    if (png) cached = png;
    return png;
  } catch {
    return null;
  }
}
