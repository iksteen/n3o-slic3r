import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import * as THREE from "three";
import { onEvents } from "../state/eventRouter";
import { shouldIgnoreHotkey } from "../ui/hotkeyInhibit";
import type { SceneObject } from "./types";

type Vec3 = [number, number, number];
type GizmoMode = "none" | "move" | "rotate" | "scale";
type GizmoInfo = { center: Vec3; length: number };
const IDENTITY16 = new THREE.Matrix4().toArray();
const IDENT_QUAT: [number, number, number, number] = [0, 0, 0, 1];
// Move/Scale gizmo handle length as a fraction of the eye→gizmo distance
// (constant on-screen size). Must match GIZMO_SCREEN_K in viewport_render.rs.
const GIZMO_SCREEN_K = 0.13;

type DragState = {
  x: number;
  y: number;
  button: number;
  mode: "pending" | "orbit" | "move" | "rotate" | "scale" | "pan" | "inert";
  moveTargets?: { id: number; start: number[] }[];
  // Constrained move: intersect the cursor ray with this fixed plane; with
  // `axisDir` set, keep only the component along it (single-axis handle),
  // otherwise use the full in-plane delta (planar handle / free XY drag).
  planeN?: Vec3;
  planeP?: Vec3;
  startHit?: Vec3 | null;
  axisDir?: Vec3 | null;
  // Constrained rotate: signed angle of the cursor about `rotAxis` through the
  // `pivot`, in the ring's plane (normal = rotAxis).
  rotAxis?: Vec3;
  pivot?: Vec3;
  // Scale: cursor displacement along the handle's gesture direction (over the
  // handle length) drives a factor on the selected local axes (`scaleMask`) or
  // all of them (`uniform`). `basisAxes` are the gizmo's world axes at drag start.
  scaleMask?: [boolean, boolean, boolean];
  uniform?: boolean;
  basisQuat?: [number, number, number, number];
  basisAxes?: [Vec3, Vec3, Vec3];
};

const sub = (a: Vec3, b: Vec3): Vec3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const addv = (a: Vec3, b: Vec3): Vec3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const scale = (a: Vec3, k: number): Vec3 => [a[0] * k, a[1] * k, a[2] * k];
const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: Vec3, b: Vec3): Vec3 => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
const vlen = (a: Vec3) => Math.hypot(a[0], a[1], a[2]);
const norm = (a: Vec3): Vec3 => {
  const l = vlen(a) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
};

// Gizmo handle layout — must mirror gizmo_geometry() in viewport_render.rs.
const AXES: Vec3[] = [
  [1, 0, 0],
  [0, 1, 0],
  [0, 0, 1],
];
const PLANES: { n: Vec3; a: Vec3; b: Vec3 }[] = [
  { n: [0, 0, 1], a: [1, 0, 0], b: [0, 1, 0] }, // XY
  { n: [1, 0, 0], a: [0, 1, 0], b: [0, 0, 1] }, // YZ
  { n: [0, 1, 0], a: [1, 0, 0], b: [0, 0, 1] }, // XZ
];

/**
 * Strategy-A wgpu viewport (Linux, `N3O_WGPU=1`): the 3D scene is rendered in
 * Rust (wgpu, offscreen) and the finished frame is blitted into this opaque
 * `<canvas>`. WebKitGTK can't composite a transparent webview over GPU content
 * (it smears dynamic DOM — see docs/dev/wgpu-renderer.md), so the render comes
 * to the webview as pixels rather than the webview sitting over a GL surface.
 *
 * Navigation: left-drag orbits, right-drag pans, wheel zooms. Click selects via
 * a Rust ray-cast. With a selection, pressing on the selected object(s) and
 * dragging moves it on the X,Y plane; in gizmo move/rotate mode the body never
 * drags — the axis/plane handles (move) or axis rings (rotate) drive constrained
 * transforms instead. Frames render on-demand and coalesce (one in flight).
 */
type Tool = "none" | "layflat" | "alignX" | "alignY" | "facematch";

export function WgpuViewport({
  objects,
  selectedIds,
  gizmoMode = "none",
  tool = "none",
  onToolDone,
}: {
  objects: SceneObject[];
  selectedIds: number[];
  gizmoMode?: GizmoMode;
  tool?: Tool;
  onToolDone?: () => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const cam = useRef({ az: 0.9, el: 0.6, dist: 350, center: [0, 0, 0] as [number, number, number] });
  // Kept fresh each render so the (mount-once) effect's handlers see live values.
  const objectsRef = useRef(objects);
  objectsRef.current = objects;
  const selRef = useRef(selectedIds);
  selRef.current = selectedIds;
  const gizmoModeRef = useRef(gizmoMode);
  gizmoModeRef.current = gizmoMode;
  const toolRef = useRef(tool);
  toolRef.current = tool;
  const onToolDoneRef = useRef(onToolDone);
  onToolDoneRef.current = onToolDone;
  // Face-match is a two-click pick: the first click stashes the reference face's
  // world normal + point here; the second matches the target face to it.
  const faceMatchRef = useRef<{ normal: Vec3; point: Vec3 } | null>(null);
  // Lets effects outside the mount-once effect trigger a redraw / gizmo refresh.
  const renderRef = useRef<(() => void) | null>(null);
  const refreshGizmoRef = useRef<(() => void) | null>(null);

  const drag = useRef<DragState | null>(null);
  // Local drag preview: the renderer world-pre-multiplies these ids by `pre`
  // (column-major 4x4) per frame without touching scene state; the real
  // transforms commit on release.
  const dragOverride = useRef<{ ids: number[]; pre: number[] } | null>(null);
  const inflight = useRef(false);
  const dirty = useRef(false);
  // Gizmo placement cache + the currently hovered handle (-1 = none), so idle
  // mouse-moves can hit-test handles without an IPC round-trip per frame.
  const gizmoInfoRef = useRef<GizmoInfo | null>(null);
  const hoverHandle = useRef(-1);
  // Scale gizmo orientation: a single selection scales along its own axes, multi
  // along world axes (identity). Cached with the placement, refreshed on change.
  const gizmoBasis = useRef<{ quat: [number, number, number, number]; axes: [Vec3, Vec3, Vec3] }>({
    quat: IDENT_QUAT,
    axes: [
      [1, 0, 0],
      [0, 1, 0],
      [0, 0, 1],
    ],
  });

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    async function render() {
      if (inflight.current) {
        dirty.current = true;
        return;
      }
      const cv = canvasRef.current;
      if (!cv) return;
      const w = Math.max(1, cv.clientWidth);
      const h = Math.max(1, cv.clientHeight);
      if (cv.width !== w) cv.width = w;
      if (cv.height !== h) cv.height = h;
      inflight.current = true;
      try {
        const c = cam.current;
        const ov = dragOverride.current;
        const buf = await invoke<ArrayBuffer>("viewport_frame", {
          req: {
            width: w,
            height: h,
            az: c.az,
            el: c.el,
            dist: c.dist,
            center: c.center,
            drag_ids: ov?.ids ?? [],
            drag_pre: ov?.pre ?? IDENTITY16,
            gizmo: gizmoModeRef.current,
            gizmo_basis: gizmoBasis.current.quat,
            gizmo_dragging: drag.current?.mode === "scale",
            gizmo_hover: hoverHandle.current,
          },
        });
        ctx?.putImageData(new ImageData(new Uint8ClampedArray(buf), w, h), 0, 0);
      } catch (e) {
        console.error("viewport_frame failed", e);
      } finally {
        inflight.current = false;
        if (dirty.current) {
          dirty.current = false;
          void render();
        }
      }
    }

    const rel = (e: MouseEvent): [number, number] => {
      const r = canvas.getBoundingClientRect();
      return [e.clientX - r.left, e.clientY - r.top];
    };

    // Ray-cast (Rust) → nearest hit object id, or null.
    const castPick = async (sx: number, sy: number): Promise<number | null> => {
      const cv = canvasRef.current;
      if (!cv) return null;
      const c = cam.current;
      try {
        return await invoke<number | null>("viewport_pick", {
          req: { width: cv.width, height: cv.height, x: sx, y: sy, az: c.az, el: c.el, dist: c.dist, center: c.center },
        });
      } catch (e) {
        console.error("viewport_pick failed", e);
        return null;
      }
    };
    // Modifier-click (shift/ctrl/cmd) extends: Toggle adds the clicked object's
    // whole group (or removes it if already selected); a plain click replaces.
    // An additive click on empty space keeps the selection. Mirrors ViewportCanvas.
    const applySelect = async (id: number | null, additive: boolean) => {
      try {
        if (id != null) {
          await invoke("scene_select", {
            ids: [id],
            mode: additive ? "Toggle" : "Replace",
            expandGroups: true,
          });
        } else if (!additive) {
          await invoke("scene_deselect");
        }
      } catch (e) {
        console.error("select failed", e);
      }
    };

    // Face-pick (Rust): nearest hit's object id + world face normal + hit point.
    type FacePick = { id: number; normal: Vec3; point: Vec3 };
    const castPickFace = async (sx: number, sy: number): Promise<FacePick | null> => {
      const cv = canvasRef.current;
      if (!cv) return null;
      const c = cam.current;
      try {
        return await invoke<FacePick | null>("viewport_pick_face", {
          req: { width: cv.width, height: cv.height, x: sx, y: sy, az: c.az, el: c.el, dist: c.dist, center: c.center },
        });
      } catch (e) {
        console.error("viewport_pick_face failed", e);
        return null;
      }
    };

    // Run the armed placing tool for a click at (sx,sy). Returns true when it
    // acted (so the tool disarms); false to stay armed (e.g. clicked empty space).
    const runTool = async (toolNow: Tool, sx: number, sy: number): Promise<boolean> => {
      if (toolNow === "alignX" || toolNow === "alignY") {
        const id = await castPick(sx, sy);
        if (id == null) return false;
        const axis = toolNow === "alignX" ? "X" : "Y";
        try {
          await invoke("scene_select", { ids: [id], mode: "Replace", expandGroups: true });
          await invoke("scene_object_align_axis", { ids: [id], axis, expandGroups: true });
        } catch (e) {
          console.error("align failed", e);
        }
        return true;
      }
      if (toolNow === "facematch") {
        // Two clicks: first stash the reference face, then yaw the target's group
        // so its clicked face matches the reference's heading + slide coplanar.
        const hit = await castPickFace(sx, sy);
        if (!hit) return false;
        if (!faceMatchRef.current) {
          faceMatchRef.current = { normal: hit.normal, point: hit.point };
          // Select the reference object as feedback that the click registered.
          await invoke("scene_select", { ids: [hit.id], mode: "Replace", expandGroups: true }).catch(
            (e) => console.error("select failed", e),
          );
          return false; // stay armed for the second click
        }
        const ref = faceMatchRef.current;
        faceMatchRef.current = null;
        try {
          await invoke("scene_select", { ids: [hit.id], mode: "Replace", expandGroups: true });
          await invoke("scene_object_align_face", {
            ids: [hit.id],
            refNormal: ref.normal,
            faceNormal: hit.normal,
            refPoint: ref.point,
            facePoint: hit.point,
            expandGroups: true,
          });
        } catch (e) {
          console.error("align face failed", e);
        }
        return true;
      }
      // layflat: click a face → rotate its outward normal to point down (-Z) and
      // drop the contact onto the bed. With a selection, lay the selected set flat
      // (exact, no group expand) and require the click to land on it; with none,
      // lay the clicked object's whole group flat.
      const hit = await castPickFace(sx, sy);
      if (!hit) return false;
      const sel = selRef.current;
      let ids: number[];
      let expandGroups: boolean;
      if (sel.length > 0) {
        if (!sel.includes(hit.id)) return false; // off-selection click — stay armed
        ids = sel;
        expandGroups = false;
      } else {
        ids = [hit.id];
        expandGroups = true;
      }
      const q = new THREE.Quaternion().setFromUnitVectors(
        new THREE.Vector3(...hit.normal),
        new THREE.Vector3(0, 0, -1),
      );
      try {
        await invoke("scene_object_lay_flat_on", {
          ids,
          rotation: [q.x, q.y, q.z, q.w],
          contact: hit.point,
          expandGroups,
        });
      } catch (e) {
        console.error("lay flat failed", e);
      }
      return true;
    };

    // Cursor ray in world space. A Three.js camera posed exactly like the Rust
    // one (eye/center/up/fov/aspect) gives a ray matching what's drawn.
    const makeRay = (sx: number, sy: number): THREE.Ray | null => {
      const cv = canvasRef.current;
      if (!cv) return null;
      const c = cam.current;
      const ce = Math.cos(c.el),
        se = Math.sin(c.el);
      const ca = Math.cos(c.az),
        sa = Math.sin(c.az);
      const cam3 = new THREE.PerspectiveCamera(45, cv.width / cv.height, 0.1, Math.max(1000, c.dist * 10));
      cam3.up.set(0, 0, 1);
      cam3.position.set(
        c.center[0] + c.dist * ce * ca,
        c.center[1] + c.dist * ce * sa,
        c.center[2] + c.dist * se,
      );
      cam3.lookAt(c.center[0], c.center[1], c.center[2]);
      cam3.updateMatrixWorld();
      cam3.updateProjectionMatrix();
      const rc = new THREE.Raycaster();
      rc.setFromCamera(new THREE.Vector2((2 * sx) / cv.width - 1, -((2 * sy) / cv.height - 1)), cam3);
      return rc.ray.clone();
    };
    // Intersect a ray with the plane (normal n, through point p) → world point.
    const rayPlanePoint = (ray: THREE.Ray, n: Vec3, p: Vec3): Vec3 | null => {
      const plane = new THREE.Plane().setFromNormalAndCoplanarPoint(
        new THREE.Vector3(...n),
        new THREE.Vector3(...p),
      );
      const hit = new THREE.Vector3();
      if (!ray.intersectPlane(plane, hit)) return null;
      return [hit.x, hit.y, hit.z];
    };
    const rayOrigin = (ray: THREE.Ray): Vec3 => [ray.origin.x, ray.origin.y, ray.origin.z];
    const rayDir = (ray: THREE.Ray): Vec3 => [ray.direction.x, ray.direction.y, ray.direction.z];

    // Closest distance between `ray` and segment [A,B], plus the ray parameter at
    // the closest point (used to pick the nearest handle to the camera).
    const raySegDist = (ray: THREE.Ray, A: Vec3, B: Vec3): { dist: number; t: number } => {
      const O = rayOrigin(ray);
      const D = rayDir(ray);
      const AB = sub(B, A);
      const r = sub(O, A);
      const a = dot(AB, AB),
        b = dot(AB, D),
        d = dot(AB, r),
        e = dot(D, r);
      const denom = a - b * b;
      let s = denom > 1e-9 ? (d - b * e) / denom : 0;
      s = Math.max(0, Math.min(1, s));
      const t = Math.max(0, b * s - e);
      const pc = addv(A, scale(AB, s));
      const qc = addv(O, scale(D, t));
      return { dist: vlen(sub(pc, qc)), t };
    };

    // Distance from `ray` to point `p`, plus the ray parameter at the closest
    // point (for picking the center uniform-scale handle).
    const rayPointDist = (ray: THREE.Ray, p: Vec3): { dist: number; t: number } => {
      const O = rayOrigin(ray);
      const t = Math.max(0, dot(sub(p, O), rayDir(ray)));
      const q = addv(O, scale(rayDir(ray), t));
      return { dist: vlen(sub(p, q)), t };
    };

    // idx mirrors the gizmo geometry's handle order: Move 0/1/2 = X/Y/Z axis,
    // 3/4/5 = XY/YZ/XZ plane; Rotate 0/1/2 = X/Y/Z ring; Scale 0/1/2 = X/Y/Z
    // axis, 3/4/5 = plane, 6 = center uniform. A handle carries a translate
    // constraint (planeN/planeP/axisDir), a rotate one (rotAxis), or a scale one
    // (planeN/planeP + scaleMask/uniform).
    type Handle = {
      idx: number;
      planeN?: Vec3;
      planeP?: Vec3;
      axisDir?: Vec3 | null;
      rotAxis?: Vec3;
      scaleMask?: [boolean, boolean, boolean];
      uniform?: boolean;
    };
    // Hit-test the move handles (axis rod+ball / plane quad) for `ray`.
    const pickMoveHandle = (ray: THREE.Ray, c: Vec3, l: number): { t: number; h: Handle } | null => {
      let best: { t: number; h: Handle } | null = null;
      const pickR = l * 0.14;
      AXES.forEach((dir, i) => {
        const { dist, t } = raySegDist(ray, c, addv(c, scale(dir, l)));
        if (dist < pickR && (!best || t < best.t)) {
          const view = norm(rayDir(ray));
          let n = sub(view, scale(dir, dot(view, dir)));
          if (vlen(n) < 1e-4) n = AXES[(i + 1) % 3]; // looking down the axis
          best = { t, h: { planeN: norm(n), planeP: c, axisDir: dir, idx: i } };
        }
      });
      const o = l * 0.28,
        s = l * 0.24;
      for (let i = 0; i < PLANES.length; i++) {
        const pl = PLANES[i];
        const hit = rayPlanePoint(ray, pl.n, c);
        if (!hit) continue;
        const da = dot(sub(hit, c), pl.a),
          db = dot(sub(hit, c), pl.b);
        if (da >= o && da <= o + s && db >= o && db <= o + s) {
          const t = dot(sub(hit, rayOrigin(ray)), rayDir(ray));
          if (!best || t < best.t)
            best = { t, h: { planeN: pl.n, planeP: c, axisDir: null, idx: 3 + i } };
        }
      }
      return best;
    };
    // Hit-test the rotate rings (radius l, in the plane ⟂ each axis) for `ray`.
    const pickRotateHandle = (ray: THREE.Ray, c: Vec3, l: number): { t: number; h: Handle } | null => {
      let best: { t: number; h: Handle } | null = null;
      const tol = l * 0.12;
      AXES.forEach((axis, i) => {
        const hit = rayPlanePoint(ray, axis, c);
        if (!hit) return;
        if (Math.abs(vlen(sub(hit, c)) - l) > tol) return;
        const t = dot(sub(hit, rayOrigin(ray)), rayDir(ray));
        if (!best || t < best.t) best = { t, h: { rotAxis: axis, idx: i } };
      });
      return best;
    };
    // Hit-test the scale handles (axis rod+cube / plane quad / center cube) for
    // `ray`, using the gizmo's oriented basis axes.
    const pickScaleHandle = (ray: THREE.Ray, c: Vec3, l: number): { t: number; h: Handle } | null => {
      const axes = gizmoBasis.current.axes;
      // Center uniform 3-axis handle first: the axis rods all pass through the
      // center, so without priority here one of them always wins the tie.
      const ctr = rayPointDist(ray, c);
      if (ctr.dist < l * 0.16) {
        return { t: ctr.t, h: { idx: 6, planeN: norm(rayDir(ray)), planeP: c, uniform: true } };
      }
      let best: { t: number; h: Handle } | null = null;
      const pickR = l * 0.14;
      for (let i = 0; i < axes.length; i++) {
        const dir = axes[i];
        const { dist, t } = raySegDist(ray, c, addv(c, scale(dir, l)));
        if (dist < pickR && (!best || t < best.t)) {
          const view = norm(rayDir(ray));
          let n = sub(view, scale(dir, dot(view, dir)));
          if (vlen(n) < 1e-4) n = axes[(i + 1) % 3];
          const mask: [boolean, boolean, boolean] = [i === 0, i === 1, i === 2];
          best = { t, h: { idx: i, planeN: norm(n), planeP: c, scaleMask: mask } };
        }
      }
      // Planar handles: pairs (XY, YZ, XZ) of the basis axes; normal = third.
      const planeDefs: [number, number, number][] = [
        [0, 1, 2],
        [1, 2, 0],
        [0, 2, 1],
      ];
      const o = l * 0.28,
        s = l * 0.24;
      for (let i = 0; i < planeDefs.length; i++) {
        const [ai, bi, ni] = planeDefs[i];
        const hit = rayPlanePoint(ray, axes[ni], c);
        if (!hit) continue;
        const da = dot(sub(hit, c), axes[ai]),
          db = dot(sub(hit, c), axes[bi]);
        if (da >= o && da <= o + s && db >= o && db <= o + s) {
          const t = dot(sub(hit, rayOrigin(ray)), rayDir(ray));
          const mask: [boolean, boolean, boolean] = [false, false, false];
          mask[ai] = true;
          mask[bi] = true;
          if (!best || t < best.t) best = { t, h: { idx: 3 + i, planeN: axes[ni], planeP: c, scaleMask: mask } };
        }
      }
      return best;
    };
    // Eye→point distance for the current camera (matches view_proj's eye).
    const eyeDist = (p: Vec3): number => {
      const c = cam.current;
      const ce = Math.cos(c.el),
        se = Math.sin(c.el);
      const ca = Math.cos(c.az),
        sa = Math.sin(c.az);
      const eye: Vec3 = [
        c.center[0] + c.dist * ce * ca,
        c.center[1] + c.dist * ce * sa,
        c.center[2] + c.dist * se,
      ];
      return vlen(sub(eye, p));
    };
    // Camera right vector — the uniform-scale gesture direction when the handle
    // is grabbed dead-center (no radial direction to use).
    const camRight = (): Vec3 => {
      const c = cam.current;
      const ce = Math.cos(c.el),
        se = Math.sin(c.el),
        ca = Math.cos(c.az),
        sa = Math.sin(c.az);
      const fwd: Vec3 = [-ce * ca, -ce * sa, -se]; // eye → center
      return norm(cross(fwd, [0, 0, 1]));
    };
    // Nearest gizmo handle under `ray` for the current mode, or null. Move and
    // Scale are constant on-screen size, so their hit-test length tracks the
    // camera; Rotate is sized to the object.
    const pickHandle = (ray: THREE.Ray, gi: GizmoInfo): Handle | null => {
      if (gizmoModeRef.current === "rotate") return pickRotateHandle(ray, gi.center, gi.length)?.h ?? null;
      const l = GIZMO_SCREEN_K * eyeDist(gi.center);
      if (gizmoModeRef.current === "scale") return pickScaleHandle(ray, gi.center, l)?.h ?? null;
      return pickMoveHandle(ray, gi.center, l)?.h ?? null;
    };

    // Scale gizmo orientation: a single selection scales along its own (rotated)
    // axes; multi (or none) is world-aligned. Mirrors how the renderer orients
    // the scale handles, so hit-test and drawing agree.
    const computeBasis = () => {
      const sel = selRef.current;
      if (sel.length === 1) {
        const o = objectsRef.current.find((x) => x.id === sel[0]);
        if (o) {
          const q = new THREE.Quaternion();
          new THREE.Matrix4().fromArray(o.transform).decompose(new THREE.Vector3(), q, new THREE.Vector3());
          const ax = (v: THREE.Vector3): Vec3 => {
            const r = v.applyQuaternion(q);
            return [r.x, r.y, r.z];
          };
          gizmoBasis.current = {
            quat: [q.x, q.y, q.z, q.w],
            axes: [ax(new THREE.Vector3(1, 0, 0)), ax(new THREE.Vector3(0, 1, 0)), ax(new THREE.Vector3(0, 0, 1))],
          };
          return;
        }
      }
      gizmoBasis.current = {
        quat: IDENT_QUAT,
        axes: [
          [1, 0, 0],
          [0, 1, 0],
          [0, 0, 1],
        ],
      };
    };

    // Refresh the cached gizmo placement (null when no gizmo / no selection).
    // Driven by the scene-change events, not per-frame.
    const refreshGizmo = async () => {
      if (gizmoModeRef.current === "none") {
        gizmoInfoRef.current = null;
        return;
      }
      computeBasis();
      try {
        gizmoInfoRef.current = await invoke<GizmoInfo | null>("viewport_gizmo");
      } catch (e) {
        console.error("viewport_gizmo failed", e);
        gizmoInfoRef.current = null;
      }
    };

    // Highlight the handle under the cursor while idle in a gizmo mode.
    const updateHover = (e: MouseEvent) => {
      let idx = -1;
      const gi = gizmoInfoRef.current;
      if (gi) {
        const [sx, sy] = rel(e);
        const ray = makeRay(sx, sy);
        if (ray) idx = pickHandle(ray, gi)?.idx ?? -1;
      }
      if (idx !== hoverHandle.current) {
        hoverHandle.current = idx;
        void render();
      }
    };

    // Selected objects + their starting transforms, for a move/rotate drag.
    const collectMoveTargets = () =>
      selRef.current
        .map((sid) => objectsRef.current.find((o) => o.id === sid))
        .filter((o): o is SceneObject => !!o)
        .map((o) => ({ id: o.id, start: [...o.transform] }));

    const translationMat = (t: Vec3): number[] =>
      new THREE.Matrix4().makeTranslation(t[0], t[1], t[2]).toArray();
    // World rotation of `angle` about `axis` through `pivot` (column-major).
    const pivotRotation = (axis: Vec3, angle: number, pivot: Vec3): number[] => {
      const p = new THREE.Vector3(...pivot);
      return new THREE.Matrix4()
        .makeTranslation(p.x, p.y, p.z)
        .multiply(new THREE.Matrix4().makeRotationAxis(new THREE.Vector3(...axis).normalize(), angle))
        .multiply(new THREE.Matrix4().makeTranslation(-p.x, -p.y, -p.z))
        .toArray();
    };
    // Signed angle from v0 to v1 measured about `axis` (right-hand rule).
    const signedAngle = (v0: Vec3, v1: Vec3, axis: Vec3): number => {
      const cr: Vec3 = [
        v0[1] * v1[2] - v0[2] * v1[1],
        v0[2] * v1[0] - v0[0] * v1[2],
        v0[0] * v1[1] - v0[1] * v1[0],
      ];
      return Math.atan2(dot(axis, cr), dot(v0, v1));
    };
    // World scale by `ratio` about `pivot` in the `quat` frame (column-major): for
    // a single selection that scales along the object's own axes (no shear); for
    // multi `quat` is identity so it's a world-axis scale about the center.
    const scalePre = (ratio: Vec3, pivot: Vec3, quat: [number, number, number, number]): number[] => {
      const p = new THREE.Vector3(...pivot);
      const R = new THREE.Matrix4().makeRotationFromQuaternion(new THREE.Quaternion(...quat));
      const Rinv = R.clone().transpose();
      return new THREE.Matrix4()
        .makeTranslation(p.x, p.y, p.z)
        .multiply(R)
        .multiply(new THREE.Matrix4().makeScale(ratio[0], ratio[1], ratio[2]))
        .multiply(Rinv)
        .multiply(new THREE.Matrix4().makeTranslation(-p.x, -p.y, -p.z))
        .toArray();
    };

    // Commit object transforms, coalesced (one batch in flight, latest per id).
    let commitInflight = false;
    let pending: Map<number, number[]> | null = null;
    const flushCommit = async () => {
      if (commitInflight || !pending) return;
      commitInflight = true;
      const batch = pending;
      pending = null;
      try {
        await Promise.all(
          [...batch].map(([id, transform]) => invoke("scene_object_set_transform", { id, transform })),
        );
      } catch (e) {
        console.error("set_transform failed", e);
      }
      commitInflight = false;
      if (pending) void flushCommit();
    };
    const commitMove = (updates: { id: number; m: number[] }[]) => {
      if (!pending) pending = new Map();
      for (const u of updates) pending.set(u.id, u.m);
      void flushCommit();
    };

    // Right-drag pans: move the look-at center in the view plane, scaled so the
    // grabbed point tracks the cursor (world-units-per-pixel at the focal plane).
    const pan = (dx: number, dy: number) => {
      const c = cam.current;
      const ce = Math.cos(c.el),
        se = Math.sin(c.el);
      const ca = Math.cos(c.az),
        sa = Math.sin(c.az);
      const fx = -ce * ca,
        fy = -ce * sa,
        fz = -se;
      const rl = Math.hypot(fy, fx) || 1;
      const rx = fy / rl,
        ry = -fx / rl;
      const ux = ry * fz,
        uy = -rx * fz,
        uz = rx * fy - ry * fx;
      const k = (2 * c.dist * Math.tan((45 * Math.PI) / 180 / 2)) / Math.max(1, canvas.height);
      c.center[0] += (-dx * rx + dy * ux) * k;
      c.center[1] += (-dx * ry + dy * uy) * k;
      c.center[2] += dy * uz * k;
      void render();
    };

    let moved = false; // distinguishes a drag from a click
    const onDown = (e: MouseEvent) => {
      if (e.button !== 0 && e.button !== 2) return;
      moved = false;
      if (e.button === 2) {
        e.preventDefault();
        drag.current = { x: e.clientX, y: e.clientY, button: 2, mode: "pan" };
        return;
      }
      // Left: decide orbit vs move-selection with a pick (async). Until it
      // resolves the press stays "pending" and movement is buffered.
      const press: DragState = { x: e.clientX, y: e.clientY, button: 0, mode: "pending" };
      drag.current = press;
      const [sx, sy] = rel(e);

      if (toolRef.current !== "none") {
        // A placing tool is armed: drags orbit, the click (in onUp) runs the tool.
        press.mode = "orbit";
        return;
      }

      if (gizmoModeRef.current !== "none") {
        // Gizmo mode: only the handles drive transforms; the body never drags.
        void invoke<GizmoInfo | null>("viewport_gizmo").then(async (gi) => {
          if (drag.current !== press) return;
          gizmoInfoRef.current = gi;
          const ray = makeRay(sx, sy);
          const handle = gi && ray ? pickHandle(ray, gi) : null;
          if (handle && ray) {
            press.moveTargets = collectMoveTargets();
            hoverHandle.current = handle.idx; // keep it lit through the drag
            if (handle.rotAxis) {
              press.mode = "rotate";
              press.rotAxis = handle.rotAxis;
              press.pivot = gi!.center;
              press.startHit = rayPlanePoint(ray, handle.rotAxis, gi!.center);
            } else if (handle.scaleMask || handle.uniform) {
              press.mode = "scale";
              press.planeN = handle.planeN;
              press.planeP = handle.planeP;
              press.pivot = gi!.center;
              press.scaleMask = handle.scaleMask;
              press.uniform = handle.uniform;
              press.basisQuat = gizmoBasis.current.quat;
              press.basisAxes = gizmoBasis.current.axes;
              press.startHit = rayPlanePoint(ray, handle.planeN!, handle.planeP!);
            } else {
              press.mode = "move";
              press.planeN = handle.planeN;
              press.planeP = handle.planeP;
              press.axisDir = handle.axisDir;
              press.startHit = rayPlanePoint(ray, handle.planeN!, handle.planeP!);
            }
            return;
          }
          // Missed every handle: pressing the selected body does nothing (the
          // handles drive transforms); pressing empty space orbits to navigate.
          const id = await castPick(sx, sy);
          if (drag.current !== press) return;
          press.mode = id != null && selRef.current.includes(id) ? "inert" : "orbit";
        });
        return;
      }

      // No gizmo: pressing a selected object free-moves the selection on its XY plane.
      void castPick(sx, sy).then((id) => {
        if (drag.current !== press) return; // released or superseded
        if (id != null && selRef.current.includes(id)) {
          const hit = objectsRef.current.find((o) => o.id === id);
          const planeZ = hit?.transform[14] ?? 0;
          const ray = makeRay(sx, sy);
          press.mode = "move";
          press.planeN = [0, 0, 1];
          press.planeP = [0, 0, planeZ];
          press.axisDir = null;
          press.startHit = ray ? rayPlanePoint(ray, [0, 0, 1], [0, 0, planeZ]) : null;
          press.moveTargets = collectMoveTargets();
        } else {
          press.mode = "orbit";
        }
      });
    };
    const onUp = (e: MouseEvent) => {
      const d = drag.current;
      drag.current = null;
      if (!d || d.button !== 0) return;
      if (d.mode === "move" || d.mode === "rotate" || d.mode === "scale") {
        // Commit the previewed transform once (new = pre * start). Clear the
        // override first; the object_updated event then re-renders at the
        // (identical) committed pose — the on-screen preview never jumps.
        const ov = dragOverride.current;
        dragOverride.current = null;
        if (ov && d.moveTargets) {
          const pre = new THREE.Matrix4().fromArray(ov.pre);
          commitMove(
            d.moveTargets.map((t) => ({
              id: t.id,
              m: pre.clone().multiply(new THREE.Matrix4().fromArray(t.start)).toArray(),
            })),
          );
        }
        return;
      }
      if (moved) return;
      const [sx, sy] = rel(e);
      // A placing tool is armed: the click runs it (and disarms on success);
      // otherwise it's a normal selection click.
      if (toolRef.current !== "none") {
        const t = toolRef.current;
        void runTool(t, sx, sy).then((acted) => {
          if (acted) onToolDoneRef.current?.();
        });
        return;
      }
      // A click (no drag) on empty space or an unselected object selects it; a
      // click on an already-selected object keeps the selection. Shift/ctrl/cmd
      // extends the selection instead of replacing it.
      const additive = e.shiftKey || e.metaKey || e.ctrlKey;
      void castPick(sx, sy).then((id) => applySelect(id, additive));
    };
    const onMove = (e: MouseEvent) => {
      const d = drag.current;
      if (!d) {
        if (gizmoModeRef.current !== "none") updateHover(e); // idle handle highlight
        return;
      }
      const dx = e.clientX - d.x;
      const dy = e.clientY - d.y;
      if (Math.abs(dx) + Math.abs(dy) > 3) moved = true;
      d.x = e.clientX;
      d.y = e.clientY;
      if (d.mode === "pan") {
        pan(dx, dy);
      } else if (d.mode === "orbit") {
        const c = cam.current;
        c.az -= dx * 0.01;
        c.el = Math.min(1.45, Math.max(-1.45, c.el + dy * 0.01));
        void render();
      } else if (d.mode === "move" && d.moveTargets && d.startHit && d.planeN && d.planeP) {
        const [sx, sy] = rel(e);
        const ray = makeRay(sx, sy);
        if (!ray) return;
        const hit = rayPlanePoint(ray, d.planeN, d.planeP);
        if (!hit) return;
        let t = sub(hit, d.startHit);
        if (d.axisDir) t = scale(d.axisDir, dot(t, d.axisDir)); // single-axis only
        // Preview-only: pre-multiply the dragged objects this frame; no commit yet.
        dragOverride.current = { ids: d.moveTargets.map((x) => x.id), pre: translationMat(t) };
        void render();
      } else if (d.mode === "rotate" && d.moveTargets && d.startHit && d.rotAxis && d.pivot) {
        const [sx, sy] = rel(e);
        const ray = makeRay(sx, sy);
        if (!ray) return;
        const hit = rayPlanePoint(ray, d.rotAxis, d.pivot);
        if (!hit) return;
        const angle = signedAngle(sub(d.startHit, d.pivot), sub(hit, d.pivot), d.rotAxis);
        dragOverride.current = {
          ids: d.moveTargets.map((x) => x.id),
          pre: pivotRotation(d.rotAxis, angle, d.pivot),
        };
        void render();
      } else if (
        d.mode === "scale" &&
        d.moveTargets &&
        d.startHit &&
        d.planeN &&
        d.planeP &&
        d.pivot &&
        d.basisQuat &&
        d.basisAxes
      ) {
        const [sx, sy] = rel(e);
        const ray = makeRay(sx, sy);
        if (!ray) return;
        const hit = rayPlanePoint(ray, d.planeN, d.planeP);
        if (!hit) return;
        const mask: [boolean, boolean, boolean] = d.uniform ? [true, true, true] : d.scaleMask!;
        const [ax, ay, az] = d.basisAxes;
        let f: number;
        if (d.uniform) {
          // The center handle has no directional anchor (grabbed at the pivot),
          // so it's a zoom: factor doubles/halves per handle-length of drag.
          const radial = sub(d.startHit, d.pivot);
          const g = vlen(radial) > 1e-3 ? norm(radial) : camRight();
          const l = GIZMO_SCREEN_K * eyeDist(d.pivot);
          f = Math.pow(2, dot(sub(hit, d.startHit), g) / l);
        } else {
          // 1:1 — the grabbed point tracks the cursor: a point at startProj along
          // the gesture direction scales to sit where the cursor now projects.
          // The axis/plane handles are offset from the pivot, so startProj is a
          // safe (non-zero) reference.
          const g = norm([
            (mask[0] ? ax[0] : 0) + (mask[1] ? ay[0] : 0) + (mask[2] ? az[0] : 0),
            (mask[0] ? ax[1] : 0) + (mask[1] ? ay[1] : 0) + (mask[2] ? az[1] : 0),
            (mask[0] ? ax[2] : 0) + (mask[1] ? ay[2] : 0) + (mask[2] ? az[2] : 0),
          ]);
          const startProj = dot(sub(d.startHit, d.pivot), g);
          const curProj = dot(sub(hit, d.pivot), g);
          const ref = Math.abs(startProj) > 1e-3 ? startProj : GIZMO_SCREEN_K * eyeDist(d.pivot);
          f = curProj / ref;
        }
        f = Math.max(0.01, f); // never collapse to zero / mirror
        const ratio: Vec3 = [mask[0] ? f : 1, mask[1] ? f : 1, mask[2] ? f : 1];
        dragOverride.current = {
          ids: d.moveTargets.map((x) => x.id),
          pre: scalePre(ratio, d.pivot, d.basisQuat),
        };
        void render();
      }
      // mode === "pending": waiting on the pick — next move acts.
    };
    const onCtxMenu = (e: MouseEvent) => e.preventDefault(); // right-drag pans, no menu
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const c = cam.current;
      c.dist = Math.min(2000, Math.max(20, c.dist * (1 + Math.sign(e.deltaY) * 0.1)));
      void render();
    };

    // Pull the bed-framed camera (center + distance) from the backend, then draw.
    async function reframe() {
      try {
        const info = await invoke<{ center: [number, number, number]; distance: number }>(
          "viewport_scene_info",
        );
        cam.current.center = info.center;
        cam.current.dist = info.distance;
      } catch (e) {
        console.error("viewport_scene_info failed", e);
      }
      void render();
    }

    // Delete / Backspace removes the current selection (off while a modal or
    // text field has focus, so editing a field doesn't delete objects).
    const onKeyDown = (e: KeyboardEvent) => {
      if (shouldIgnoreHotkey(e)) return;
      if (e.key === "Escape" && toolRef.current !== "none") {
        onToolDoneRef.current?.(); // cancel the armed placing tool
        return;
      }
      if ((e.key === "Delete" || e.key === "Backspace") && selRef.current.length > 0) {
        void invoke("scene_object_delete", { ids: selRef.current }).catch((err) =>
          console.error("delete failed", err),
        );
      }
    };

    // Cursor left the canvas → drop any handle highlight.
    const onLeave = () => {
      if (hoverHandle.current !== -1) {
        hoverHandle.current = -1;
        void render();
      }
    };

    const ro = new ResizeObserver(() => void render());
    canvas.addEventListener("mousedown", onDown);
    window.addEventListener("mouseup", onUp);
    window.addEventListener("mousemove", onMove);
    canvas.addEventListener("mouseleave", onLeave);
    canvas.addEventListener("wheel", onWheel, { passive: false });
    canvas.addEventListener("contextmenu", onCtxMenu);
    window.addEventListener("keydown", onKeyDown);
    ro.observe(canvas);

    const offRender = onEvents(
      [
        "scene:mesh_loaded",
        "scene:object_added",
        "scene:object_updated",
        "scene:object_removed",
        "scene:selection_changed",
        "scene:material_slot_changed",
      ],
      () => {
        void refreshGizmo();
        void render();
      },
    );
    const offReframe = onEvents(
      ["scene:bed_changed", "scene:active_plate_changed"],
      () => void reframe(),
    );
    // A new project reuses MeshIds from 1, so the renderer's GPU mesh cache must
    // be dropped or it would draw the previous project's geometry. Reset, then
    // reframe (which renders) for the fresh scene.
    const offLoaded = onEvents(["project:loaded"], async () => {
      try {
        await invoke("viewport_reset");
      } catch (e) {
        console.error("viewport_reset failed", e);
      }
      void reframe();
    });

    renderRef.current = render;
    refreshGizmoRef.current = refreshGizmo;
    void reframe();
    void refreshGizmo();

    return () => {
      renderRef.current = null;
      refreshGizmoRef.current = null;
      offRender();
      offReframe();
      offLoaded();
      canvas.removeEventListener("mousedown", onDown);
      window.removeEventListener("mouseup", onUp);
      window.removeEventListener("mousemove", onMove);
      canvas.removeEventListener("mouseleave", onLeave);
      canvas.removeEventListener("wheel", onWheel);
      canvas.removeEventListener("contextmenu", onCtxMenu);
      window.removeEventListener("keydown", onKeyDown);
      ro.disconnect();
    };
  }, []);

  // Toggling gizmo mode: refresh the cached placement, reset hover, redraw.
  useEffect(() => {
    hoverHandle.current = -1;
    void refreshGizmoRef.current?.();
    renderRef.current?.();
  }, [gizmoMode]);

  // Leaving face-match (cancel / switch / complete) drops the stashed reference.
  useEffect(() => {
    if (tool !== "facematch") faceMatchRef.current = null;
  }, [tool]);

  return <canvas ref={canvasRef} style={{ width: "100%", height: "100%", display: "block" }} />;
}
