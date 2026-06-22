import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import * as THREE from "three";
import { onEvents } from "../state/eventRouter";
import type { SceneObject } from "./types";

type DragState = {
  x: number;
  y: number;
  button: number;
  mode: "pending" | "orbit" | "move" | "pan";
  moveTargets?: { id: number; start: number[] }[];
  planeZ?: number;
  startWorld?: [number, number, number] | null;
};

/**
 * Strategy-A wgpu viewport (Linux, `N3O_WGPU=1`): the 3D scene is rendered in
 * Rust (wgpu, offscreen) and the finished frame is blitted into this opaque
 * `<canvas>`. WebKitGTK can't composite a transparent webview over GPU content
 * (it smears dynamic DOM — see docs/dev/wgpu-renderer.md), so the render comes
 * to the webview as pixels rather than the webview sitting over a GL surface.
 *
 * Navigation: left-drag orbits, right-drag pans, wheel zooms. Click selects via
 * a Rust ray-cast. With a selection, pressing on the selected object(s) and
 * dragging moves the whole selection on the X,Y plane (no gizmo mode yet).
 * Frames render on-demand and coalesce (one in flight).
 */
export function WgpuViewport({
  objects,
  selectedIds,
}: {
  objects: SceneObject[];
  selectedIds: number[];
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const cam = useRef({ az: 0.9, el: 0.6, dist: 350, center: [0, 0, 0] as [number, number, number] });
  // Kept fresh each render so the (mount-once) effect's handlers see live values.
  const objectsRef = useRef(objects);
  objectsRef.current = objects;
  const selRef = useRef(selectedIds);
  selRef.current = selectedIds;

  const drag = useRef<DragState | null>(null);
  // Local drag preview: the renderer offsets these ids by (dx,dy) per frame
  // without touching scene state; the real transforms commit on release.
  const dragOverride = useRef<{ ids: number[]; dx: number; dy: number } | null>(null);
  const inflight = useRef(false);
  const dirty = useRef(false);

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
            drag_dx: ov?.dx ?? 0,
            drag_dy: ov?.dy ?? 0,
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

    // Intersect the cursor ray with the horizontal plane z=planeZ → world point.
    // A Three.js camera posed exactly like the Rust one (eye/center/up/fov/aspect)
    // gives a matching ray.
    const rayPlaneWorld = (sx: number, sy: number, planeZ: number): [number, number, number] | null => {
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
      const hit = new THREE.Vector3();
      if (!rc.ray.intersectPlane(new THREE.Plane(new THREE.Vector3(0, 0, 1), -planeZ), hit)) return null;
      return [hit.x, hit.y, hit.z];
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
      void castPick(sx, sy).then((id) => {
        if (drag.current !== press) return; // released or superseded
        if (id != null && selRef.current.includes(id)) {
          const hit = objectsRef.current.find((o) => o.id === id);
          press.mode = "move";
          press.planeZ = hit?.transform[14] ?? 0;
          press.startWorld = rayPlaneWorld(sx, sy, press.planeZ);
          press.moveTargets = selRef.current
            .map((sid) => objectsRef.current.find((o) => o.id === sid))
            .filter((o): o is SceneObject => !!o)
            .map((o) => ({ id: o.id, start: [...o.transform] }));
        } else {
          press.mode = "orbit";
        }
      });
    };
    const onUp = (e: MouseEvent) => {
      const d = drag.current;
      drag.current = null;
      if (!d || d.button !== 0) return;
      if (d.mode === "move") {
        // Commit the previewed offset once. Clear the override first; the
        // object_updated event then re-renders at the (identical) committed
        // position — the on-screen preview never jumps.
        const ov = dragOverride.current;
        dragOverride.current = null;
        if (ov && d.moveTargets) {
          commitMove(
            d.moveTargets.map((t) => {
              const m = [...t.start];
              m[12] += ov.dx;
              m[13] += ov.dy;
              return { id: t.id, m };
            }),
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
      if (!d) return;
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
      } else if (d.mode === "move" && d.moveTargets && d.startWorld && d.planeZ != null) {
        const [sx, sy] = rel(e);
        const cur = rayPlaneWorld(sx, sy, d.planeZ);
        if (!cur) return;
        // Preview-only: offset the dragged objects this frame; no commit yet.
        dragOverride.current = {
          ids: d.moveTargets.map((t) => t.id),
          dx: cur[0] - d.startWorld[0],
          dy: cur[1] - d.startWorld[1],
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

    const ro = new ResizeObserver(() => void render());
    canvas.addEventListener("mousedown", onDown);
    window.addEventListener("mouseup", onUp);
    window.addEventListener("mousemove", onMove);
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
      () => void render(),
    );
    const offReframe = onEvents(
      ["scene:bed_changed", "scene:active_plate_changed", "project:loaded"],
      () => void reframe(),
    );

    void reframe();

    return () => {
      offRender();
      offReframe();
      canvas.removeEventListener("mousedown", onDown);
      window.removeEventListener("mouseup", onUp);
      window.removeEventListener("mousemove", onMove);
      canvas.removeEventListener("wheel", onWheel);
      canvas.removeEventListener("contextmenu", onCtxMenu);
      ro.disconnect();
    };
  }, []);

  return <canvas ref={canvasRef} style={{ width: "100%", height: "100%", display: "block" }} />;
}
