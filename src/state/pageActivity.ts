// Whether the page is currently being *painted*, and a resume signal for when
// it starts again.
//
// This exists because of a fatal asymmetry in WebKit: it suspends the JS heap's
// garbage collector while a page isn't painted, but nothing suspends the app's
// producers. A frozen page that keeps allocating never reclaims a byte, so a
// steady drip that's invisible while you're looking at it (GC keeps up, you see
// sawtooth) becomes a straight line to WebKit's ~16 GB process-kill ceiling
// while the screen is locked. That's not hypothetical: driver telemetry did
// exactly that at 235 MB/min, twice measured.
//
// The signal is a `requestAnimationFrame` heartbeat rather than
// `document.visibilityState` alone. rAF stops when the page stops being
// painted, which is precisely the condition that stops GC — whereas visibility
// is a weaker proxy: under a Wayland compositor a locked screen or an occluded
// window may leave the document "visible". Both are consulted; either one says
// frozen, we treat it as frozen.
//
// Consumers use this to *not produce* while frozen, then catch up on resume.
// Nothing here throttles a producer that only allocates in response to user
// input — a frozen page has no user input.

/** Treat the page as frozen once the heartbeat has been silent this long.
 *  Comfortably above one frame at any refresh rate, well below the point where
 *  a drip becomes a problem. */
const PAINT_TIMEOUT_MS = 2000;

let lastPaint = 0;
let started = false;
const resumeCallbacks = new Set<() => void>();

function now(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

function start(): void {
  if (started) return;
  started = true;
  lastPaint = now();
  if (typeof requestAnimationFrame !== "function") return;
  const tick = (): void => {
    const t = now();
    // A gap means we were frozen and have just come back: let consumers flush
    // whatever they held. Fires on the first painted frame after the gap.
    if (t - lastPaint > PAINT_TIMEOUT_MS) {
      for (const cb of [...resumeCallbacks]) cb();
    }
    lastPaint = t;
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

/** True while the page is being painted (and so being collected). Starts the
 *  heartbeat on first call. */
export function isPageActive(): boolean {
  start();
  if (typeof document !== "undefined" && document.visibilityState === "hidden") {
    return false;
  }
  if (typeof requestAnimationFrame !== "function") return true;
  return now() - lastPaint <= PAINT_TIMEOUT_MS;
}

/** Run `cb` on the first painted frame after a freeze. Returns an unsubscribe.
 *  Use it to flush state a consumer coalesced while frozen. */
export function onPageResume(cb: () => void): () => void {
  start();
  resumeCallbacks.add(cb);
  return () => {
    resumeCallbacks.delete(cb);
  };
}

/** Test seam: fire the resume callbacks, as the first painted frame after a
 *  freeze does. jsdom has no real frame loop to wait for. */
export function triggerPageResumeForTests(): void {
  for (const cb of [...resumeCallbacks]) cb();
}

/** Test seam: park the heartbeat `paintedAgoMs` in the past — 0 for a painted
 *  page, past [`PAINT_TIMEOUT_MS`] for a frozen one. Deliberately leaves the
 *  resume subscribers alone: consumers register theirs at module load, and
 *  dropping those would silently disable the gate under test. */
export function resetPageActivityForTests(paintedAgoMs = 0): void {
  started = true; // don't arm a real rAF loop under jsdom
  lastPaint = now() - paintedAgoMs;
}
