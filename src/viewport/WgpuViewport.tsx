import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import * as THREE from "three";
import { onEvents } from "../state/eventRouter";
import type { SceneObject } from "./types";

type Vec3 = [number, number, number];
type GizmoMode = "none" | "move" | "rotate";
type GizmoInfo = { center: Vec3; length: number };
const IDENTITY16 = new THREE.Matrix4().toArray();

type DragState = {
  x: number;
  y: number;
  button: number;
  mode: "pending" | "orbit" | "move" | "rotate" | "pan" | "inert";
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
};

const sub = (a: Vec3, b: Vec3): Vec3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const addv = (a: Vec3, b: Vec3): Vec3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const scale = (a: Vec3, k: number): Vec3 => [a[0] * k, a[1] * k, a[2] * k];
const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
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
export function WgpuViewport({
  objects,
  selectedIds,
  gizmoMode = "none",
}: {
  objects: SceneObject[];
  selectedIds: number[];
  gizmoMode?: GizmoMode;
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
    const applySelect = async (id: number | null) => {
      try {
        if (id != null) await invoke("scene_select", { ids: [id], mode: "Replace", expandGroups: true });
        else await invoke("scene_deselect");
      } catch (e) {
        console.error("select failed", e);
      }
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

    // idx mirrors the gizmo geometry's handle order: Move 0/1/2 = X/Y/Z axis,
    // 3/4/5 = XY/YZ/XZ plane; Rotate 0/1/2 = X/Y/Z ring. A handle carries either
    // a translate constraint (planeN/planeP/axisDir) or a rotate one (rotAxis).
    type Handle = {
      idx: number;
      planeN?: Vec3;
      planeP?: Vec3;
      axisDir?: Vec3 | null;
      rotAxis?: Vec3;
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
    // Nearest gizmo handle under `ray` for the current mode, or null.
    const pickHandle = (ray: THREE.Ray, gi: GizmoInfo): Handle | null => {
      const picker = gizmoModeRef.current === "rotate" ? pickRotateHandle : pickMoveHandle;
      return picker(ray, gi.center, gi.length)?.h ?? null;
    };

    // Refresh the cached gizmo placement (null when no gizmo / no selection).
    // Driven by the scene-change events, not per-frame.
    const refreshGizmo = async () => {
      if (gizmoModeRef.current === "none") {
        gizmoInfoRef.current = null;
        return;
      }
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
      if (d.mode === "move" || d.mode === "rotate") {
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
      // A click (no drag) on empty space or an unselected object selects it; a
      // click on an already-selected object keeps the selection.
      if (!moved) {
        const [sx, sy] = rel(e);
        void castPick(sx, sy).then(applySelect);
      }
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
      ro.disconnect();
    };
  }, []);

  // Toggling gizmo mode: refresh the cached placement, reset hover, redraw.
  useEffect(() => {
    hoverHandle.current = -1;
    void refreshGizmoRef.current?.();
    renderRef.current?.();
  }, [gizmoMode]);

  return <canvas ref={canvasRef} style={{ width: "100%", height: "100%", display: "block" }} />;
}
