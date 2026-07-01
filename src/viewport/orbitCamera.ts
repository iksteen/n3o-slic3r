// Per-plate orbit cameras for the two 3D canvases — the prepare viewport
// (WgpuViewport) and the g-code preview (GcodePreview). Each is a separate
// component that unmounts when you switch modes, and the view is meant to be
// remembered per plate, so a per-instance camera would be wrong twice over.
// Holding them here (module state, keyed by plate id) makes the view:
//   - carry across prepare↔preview switches (both canvases read the same cam
//     for the active plate),
//   - carry across remounts,
//   - restore per plate-tab (each plate keeps its own cam).
// This is session-only view state — deliberately NOT persisted in the project.
//
// `reframe()` in each canvas repositions its cam only on that cam's first
// framing (or a forced refit on project load); every other trigger refreshes
// bed/tower state but leaves the cam alone, so the view is retained.

export type Vec3 = [number, number, number];

export interface OrbitCam {
  az: number;
  el: number;
  dist: number;
  center: Vec3;
  /** Whether this plate's cam has been framed to content once. */
  framed: boolean;
}

// Default framing: az = -90° (X axis along the bottom), ~37° elevation.
function makeCam(): OrbitCam {
  return {
    az: -Math.PI / 2,
    el: Math.atan2(200, 260),
    dist: 350,
    center: [0, 0, 0],
    framed: false,
  };
}

// Keyed by plate id; `null` (no active plate) folds to a shared slot.
const cams = new Map<number, OrbitCam>();

/** The orbit cam for a plate, created on first use. */
export function camFor(plateId: number | null): OrbitCam {
  const key = plateId ?? -1;
  let c = cams.get(key);
  if (!c) {
    c = makeCam();
    cams.set(key, c);
  }
  return c;
}

/** True until this cam has been framed once — its first canvas fits content;
 *  later mounts / tab returns restore the retained view. */
export function needsInitialFrame(c: OrbitCam): boolean {
  return !c.framed;
}

export function markFramed(c: OrbitCam): void {
  c.framed = true;
}
