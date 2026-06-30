import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

// Owns the split tool's cutting-plane session — a transient editing tool, like
// the gizmo mode, so it lives frontend-side and never enters core/scene. The
// plane pose (origin + Euler rotation) feeds the renderer (draw plane + tint)
// and the `scene_cut_apply` command; nothing mutates the scene until the cut is
// applied. Mutually exclusive with the transform gizmo (coordinated in App).

type Vec3 = [number, number, number];

// Default plane orientation: normal along +X (the YZ plane) — splits the
// selection left/right. The 3 sliders rotate from here.
const DEFAULT_NORMAL: Vec3 = [1, 0, 0];

/** Rotate `v` by Euler XYZ angles (radians), applied Rx then Ry then Rz. */
function eulerRotate([rx, ry, rz]: Vec3, v: Vec3): Vec3 {
  const cx = Math.cos(rx),
    sx = Math.sin(rx);
  const cy = Math.cos(ry),
    sy = Math.sin(ry);
  const cz = Math.cos(rz),
    sz = Math.sin(rz);
  let [x, y, z] = v;
  [y, z] = [y * cx - z * sx, y * sx + z * cx]; // Rx
  [x, z] = [x * cy + z * sy, -x * sy + z * cy]; // Ry
  [x, y] = [x * cz - y * sz, x * sz + y * cz]; // Rz
  return [x, y, z];
}

export interface SplitSession {
  active: boolean;
  /** Plane center (world mm), moved by the gizmo drag. */
  origin: Vec3;
  /** Euler XYZ rotation (radians), driven by the 3 sliders. */
  rot: Vec3;
  /** Selection bounding-sphere radius — sizes the plane quad + gizmo arm. */
  radius: number;
  /** Keep the +normal (blue) / −normal (red) half. */
  keepPos: boolean;
  keepNeg: boolean;
  /** Plane normal derived from `rot` (default +X). */
  normal: Vec3;
  /** Start the tool for the current selection (no-op if empty). */
  enter: (selection: number[]) => void;
  exit: () => void;
  setRot: (axis: 0 | 1 | 2, rad: number) => void;
  setOrigin: (o: Vec3) => void;
  toggleKeep: (side: "pos" | "neg") => void;
  /** Re-fetch the plane center after the selection changed (while active). */
  recenter: (selection: number[]) => void;
}

export function useSplitSession(): SplitSession {
  const [active, setActive] = useState(false);
  const [origin, setOrigin] = useState<Vec3>([0, 0, 0]);
  const [rot, setRotState] = useState<Vec3>([0, 0, 0]);
  const [radius, setRadius] = useState(20);
  const [keepPos, setKeepPos] = useState(true);
  const [keepNeg, setKeepNeg] = useState(true);

  const normal = useMemo(() => eulerRotate(rot, DEFAULT_NORMAL), [rot]);

  // Seed the plane center + size from the current selection's gizmo box.
  const fetchCenter = (selection: number[]): void => {
    if (selection.length === 0) return;
    void invoke<{ center: Vec3; length: number } | null>("viewport_gizmo")
      .then((g) => {
        if (g) {
          setOrigin(g.center);
          setRadius(g.length);
        }
      })
      .catch((e) => console.error("[split] viewport_gizmo failed", e));
  };

  const enter = (selection: number[]): void => {
    if (selection.length === 0) return;
    setRotState([0, 0, 0]);
    setKeepPos(true);
    setKeepNeg(true);
    setActive(true);
    fetchCenter(selection);
  };
  const exit = (): void => setActive(false);
  const setRot = (axis: 0 | 1 | 2, rad: number): void =>
    setRotState((r) => {
      const n: Vec3 = [...r];
      n[axis] = rad;
      return n;
    });
  const toggleKeep = (side: "pos" | "neg"): void =>
    side === "pos" ? setKeepPos((v) => !v) : setKeepNeg((v) => !v);
  const recenter = (selection: number[]): void => {
    if (active) fetchCenter(selection);
  };

  return {
    active,
    origin,
    rot,
    radius,
    keepPos,
    keepNeg,
    normal,
    enter,
    exit,
    setRot,
    setOrigin,
    toggleKeep,
    recenter,
  };
}
