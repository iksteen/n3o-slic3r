import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

// Owns the split tool's cutting-plane session — a transient editing tool, like
// the gizmo mode, so it lives frontend-side and never enters core/scene. The
// plane pose (origin + Euler rotation) feeds the renderer (draw plane + tint)
// and the `scene_cut_apply` command; nothing mutates the scene until the cut is
// applied. Mutually exclusive with the transform gizmo (coordinated in App).

type Vec3 = [number, number, number];

// Default plane orientation: normal along -X (the YZ plane) — splits the
// selection left/right. Normal toward -X puts the positive (blue) half on the
// left and the negative (red) half on the right, matching the panel's
// blue-then-red list. The 3 sliders rotate from here.
const DEFAULT_NORMAL: Vec3 = [-1, 0, 0];

export type ConnectorType = "plug" | "dowel" | "snap";
export type ConnectorStyle = "prism" | "frustum";
export type ConnectorShape = "triangle" | "square" | "hexagon" | "circle";

/** Reassembly-connector params (shared shape for the default + each placed
 *  connector). Sizes/tolerance in mm. `tol` widens the hole (radius + depth) for
 *  the fit. */
export interface ConnectorParams {
  type: ConnectorType;
  style: ConnectorStyle;
  shape: ConnectorShape;
  radius: number;
  height: number;
  tol: number;
}

/** A connector placed on the cut plane: `(u, v)` in the plane's in-plane basis
 *  (mm) and its own params. */
export interface PlacedConnector {
  u: number;
  v: number;
  params: ConnectorParams;
}

const DEFAULT_PARAMS: ConnectorParams = {
  type: "plug",
  style: "prism",
  shape: "circle",
  radius: 2.5,
  height: 8,
  tol: 0.1,
};

const cross3 = (a: Vec3, b: Vec3): Vec3 => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
const normalize3 = (a: Vec3): Vec3 => {
  const l = Math.hypot(a[0], a[1], a[2]) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
};

/** The plane's in-plane orthonormal basis from its normal — MUST match the
 *  Rust renderer/command up-pick so placement (u,v) maps to the same world
 *  point the cut uses. */
export function planeBasis(n: Vec3): { e1: Vec3; e2: Vec3 } {
  const up: Vec3 = Math.abs(n[0]) > 0.9 ? [0, 1, 0] : [1, 0, 0];
  const e1 = normalize3(cross3(n, up));
  const e2 = normalize3(cross3(n, e1));
  return { e1, e2 };
}

/** World point of a plane-space (u, v). */
export function worldOf(
  u: number,
  v: number,
  origin: Vec3,
  basis: { e1: Vec3; e2: Vec3 },
): Vec3 {
  return [
    origin[0] + u * basis.e1[0] + v * basis.e2[0],
    origin[1] + u * basis.e1[1] + v * basis.e2[1],
    origin[2] + u * basis.e1[2] + v * basis.e2[2],
  ];
}

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

  // ---- connectors (joints) ----
  connectors: PlacedConnector[];
  /** Index of the selected connector, or `null`. */
  selectedConnector: number | null;
  /** "Add connector" armed — the next plane click places one. */
  placing: boolean;
  /** Params bound to the panel editor: the selected connector's, else the
   *  default seed for new placements. */
  editParams: ConnectorParams;
  setPlacing: (on: boolean) => void;
  addConnector: (u: number, v: number) => void;
  moveConnector: (i: number, u: number, v: number) => void;
  removeConnector: (i: number) => void;
  selectConnector: (i: number | null) => void;
  /** Patch the edit target (selected connector if any, else the default). */
  setParams: (patch: Partial<ConnectorParams>) => void;
  /** The connector list in the `scene_cut_apply` wire shape. */
  connectorsForApply: () => Array<Record<string, unknown>>;
}

export function useSplitSession(): SplitSession {
  const [active, setActive] = useState(false);
  const [origin, setOrigin] = useState<Vec3>([0, 0, 0]);
  const [rot, setRotState] = useState<Vec3>([0, 0, 0]);
  const [radius, setRadius] = useState(20);
  const [keepPos, setKeepPos] = useState(true);
  const [keepNeg, setKeepNeg] = useState(true);
  const [connectors, setConnectors] = useState<PlacedConnector[]>([]);
  const [selectedConnector, setSelectedConnector] = useState<number | null>(null);
  const [placing, setPlacing] = useState(false);
  const [defaultParams, setDefaultParams] = useState<ConnectorParams>(DEFAULT_PARAMS);

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
    setConnectors([]);
    setSelectedConnector(null);
    setPlacing(false);
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

  // ---- connectors ----
  const addConnector = (u: number, v: number): void => {
    setConnectors((cs) => {
      const next = [...cs, { u, v, params: { ...defaultParams } }];
      setSelectedConnector(next.length - 1);
      return next;
    });
  };
  const moveConnector = (i: number, u: number, v: number): void =>
    setConnectors((cs) => cs.map((c, k) => (k === i ? { ...c, u, v } : c)));
  const removeConnector = (i: number): void => {
    setConnectors((cs) => cs.filter((_, k) => k !== i));
    setSelectedConnector((s) => (s === i ? null : s != null && s > i ? s - 1 : s));
  };
  const selectConnector = (i: number | null): void => setSelectedConnector(i);
  const setParams = (patch: Partial<ConnectorParams>): void => {
    // The default seed always tracks the latest edit; if a connector is
    // selected, patch it too (so editing the selection is live).
    setDefaultParams((p) => ({ ...p, ...patch }));
    if (selectedConnector != null) {
      setConnectors((cs) =>
        cs.map((c, k) =>
          k === selectedConnector ? { ...c, params: { ...c.params, ...patch } } : c,
        ),
      );
    }
  };
  const editParams =
    selectedConnector != null && connectors[selectedConnector]
      ? connectors[selectedConnector].params
      : defaultParams;
  const connectorsForApply = (): Array<Record<string, unknown>> =>
    connectors.map((c) => ({
      u: c.u,
      v: c.v,
      radius: c.params.radius,
      height: c.params.height,
      r_tol: c.params.tol,
      h_tol: c.params.tol,
      type: c.params.type,
      style: c.params.style,
      shape: c.params.shape,
    }));

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
    connectors,
    selectedConnector,
    placing,
    editParams,
    setPlacing,
    addConnector,
    moveConnector,
    removeConnector,
    selectConnector,
    setParams,
    connectorsForApply,
  };
}
