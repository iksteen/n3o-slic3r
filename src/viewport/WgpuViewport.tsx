import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { onEvents } from "../state/eventRouter";
import { shouldIgnoreHotkey } from "../ui/hotkeyInhibit";
import { registerThumbnailCapture } from "./thumbnailCapture";
import { registerAxisView, type AxisView } from "./cameraControl";

type Vec3 = [number, number, number];
type GizmoMode = "none" | "move" | "rotate" | "scale";
// The grab returned by `viewport_grab` — opaque to the frontend (all the gizmo
// math is Rust-side); it carries the handle `idx` for the hover highlight and is
// passed back verbatim to drive the drag preview + commit.
type GizmoGrab = { idx: number; [k: string]: unknown };
type GrabResult =
  | { kind: "orbit" }
  | { kind: "inert" }
  | { kind: "empty" }
  | { kind: "gizmo"; grab: GizmoGrab };
// Orbit elevation limit — just shy of straight down/up (±90°). Stops a hair
// before the pole so world-Z up never degenerates (the renderer + pick ray both
// assume Z up); close enough to read as fully overhead / underneath.
const EL_LIMIT = Math.PI / 2 - 0.001;

type DragState = {
  x: number;
  y: number;
  button: number;
  mode: "pending" | "orbit" | "gizmo" | "pan" | "inert" | "tower";
  // Gizmo drag (move/rotate/scale, and the no-tool free-move): the opaque grab
  // captured on press + the latest cursor (canvas px) + Shift. The renderer turns
  // these into the preview transform Rust-side (`gizmo_drag`); the same call on
  // release (`viewport_gizmo_commit`) returns the transforms to commit.
  grab?: GizmoGrab;
  sx?: number;
  sy?: number;
  shift?: boolean;
  // Tower drag: `towerOffset` is the grab offset (bed point − corner) from
  // viewport_tower_grab; `towerClamped` is the latest Rust-clamped corner (for
  // the commit); `towerMoved` gates the commit so a click without a drag doesn't
  // pin a wipe_tower override.
  towerOffset?: [number, number];
  towerClamped?: [number, number];
  towerMoved?: boolean;
};

const sub = (a: Vec3, b: Vec3): Vec3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
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

/**
 * Strategy-A wgpu viewport: the 3D scene is rendered in Rust (wgpu, offscreen)
 * and the finished frame is blitted into this opaque `<canvas>`. WebKitGTK can't
 * composite a transparent webview over GPU content
 * (it smears dynamic DOM — see docs/dev/wgpu-renderer.md), so the render comes
 * to the webview as pixels rather than the webview sitting over a GL surface.
 *
 * Navigation: left-drag orbits, right-drag pans, wheel zooms. Click selects via
 * a Rust ray-cast. With a selection, pressing on the selected object(s) and
 * dragging moves it on the X,Y plane; in gizmo move/rotate mode the body never
 * drags — the axis/plane handles (move) or axis rings (rotate) drive constrained
 * transforms instead. Frames render on-demand and coalesce (one in flight).
 */
type Tool = "none" | "layflat" | "alignX" | "alignY" | "facematch" | "clone";

export function WgpuViewport({
  selectedIds,
  activePlateId = null,
  gizmoMode = "none",
  tool = "none",
  onToolDone,
  onClonePick,
  onFaceMatchStep,
}: {
  selectedIds: number[];
  activePlateId?: number | null;
  gizmoMode?: GizmoMode;
  tool?: Tool;
  onToolDone?: () => void;
  onClonePick?: (id: number) => void;
  /** Match-face: reference face clicked (true) → waiting on the target. */
  onFaceMatchStep?: (refSet: boolean) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  // Default framing: camera at (0, -260, 200) looking at the
  // origin (cameraControls.ts). Zero X offset → screen-right is world +X, so the
  // bed's front edge (the X axis) is horizontal along the bottom and the origin
  // sits at the lower-left; az = -90°, el = the ~37° elevation.
  const cam = useRef({
    az: -Math.PI / 2,
    el: Math.atan2(200, 260),
    dist: 350,
    center: [0, 0, 0] as [number, number, number],
  });
  // Kept fresh each render so the (mount-once) effect's handlers see live values.
  const selRef = useRef(selectedIds);
  selRef.current = selectedIds;
  const gizmoModeRef = useRef(gizmoMode);
  gizmoModeRef.current = gizmoMode;
  const toolRef = useRef(tool);
  toolRef.current = tool;
  const onToolDoneRef = useRef(onToolDone);
  onToolDoneRef.current = onToolDone;
  const onClonePickRef = useRef(onClonePick);
  onClonePickRef.current = onClonePick;
  const onFaceMatchStepRef = useRef(onFaceMatchStep);
  onFaceMatchStepRef.current = onFaceMatchStep;
  const activePlateIdRef = useRef(activePlateId);
  activePlateIdRef.current = activePlateId;
  // Face-match is a two-click pick: the first click stashes the reference face's
  // world normal + point here; the second matches the target face to it.
  const faceMatchRef = useRef<{ normal: Vec3; point: Vec3 } | null>(null);
  // Lets effects outside the mount-once effect trigger a redraw.
  const renderRef = useRef<(() => void) | null>(null);

  const drag = useRef<DragState | null>(null);
  const inflight = useRef(false);
  const dirty = useRef(false);
  // The currently hovered gizmo handle (-1 = none), driven by `viewport_grab` on
  // idle mouse-moves; sent to the renderer so it brightens that handle.
  const hoverHandle = useRef(-1);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // The whole priming tower lives Rust-side now: the renderer resolves its
    // placement per plate (drawn from `frame`, following the active plate), the
    // sliced mesh is fed by the slice sink, and the grab/clamp/footprint are in
    // viewport_tower_grab / viewport_move_tower. The frontend only forwards
    // pointer input. `bedMin` is the bed-plane Z for the drag's ray-cast.
    let bedMin: Vec3 = [0, 0, 0];
    const fmtCoord = (v: number): string => (Math.round(v * 10) / 10).toString();

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
        // Active handle drag → let the renderer resolve the preview transform from
        // the grab + cursor (Rust-side); otherwise no drag.
        const d = drag.current;
        const gizmoDrag =
          d && d.mode === "gizmo" && d.grab
            ? { grab: d.grab, sx: d.sx ?? 0, sy: d.sy ?? 0, shift: d.shift ?? false }
            : null;
        const buf = await invoke<ArrayBuffer>("viewport_frame", {
          req: {
            width: w,
            height: h,
            az: c.az,
            el: c.el,
            dist: c.dist,
            center: c.center,
            gizmo: gizmoModeRef.current,
            gizmo_drag: gizmoDrag,
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

    // Ray-cast (Rust): build the pick request from the current camera and invoke
    // `cmd`. `viewport_pick` → object id; `viewport_pick_face` → FacePick.
    const cast = async <T,>(cmd: string, sx: number, sy: number): Promise<T | null> => {
      const cv = canvasRef.current;
      if (!cv) return null;
      const c = cam.current;
      try {
        return await invoke<T>(cmd, {
          req: { width: cv.width, height: cv.height, x: sx, y: sy, az: c.az, el: c.el, dist: c.dist, center: c.center },
        });
      } catch (e) {
        console.error(`${cmd} failed`, e);
        return null;
      }
    };
    const castPick = (sx: number, sy: number) => cast<number>("viewport_pick", sx, sy);
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
    const castPickFace = (sx: number, sy: number) => cast<FacePick>("viewport_pick_face", sx, sy);

    // Run the armed placing tool for a click at (sx,sy). Returns true when it
    // acted (so the tool disarms); false to stay armed (e.g. clicked empty space).
    const runTool = async (toolNow: Tool, sx: number, sy: number): Promise<boolean> => {
      if (toolNow === "clone") {
        // Pick an object → hand its id up so App opens the clone dialog on its
        // whole group.
        const id = await castPick(sx, sy);
        if (id == null) return false;
        onClonePickRef.current?.(id);
        return true;
      }
      if (toolNow === "alignX" || toolNow === "alignY") {
        const id = await castPick(sx, sy);
        if (id == null) return false;
        const axis = toolNow === "alignX" ? "X" : "Y";
        try {
          // Act on the clicked group (expandGroups); leave the selection as-is.
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
          onFaceMatchStepRef.current?.(true); // prompt now asks for the target face
          return false; // stay armed for the second click
        }
        const ref = faceMatchRef.current;
        faceMatchRef.current = null;
        try {
          // Act on the clicked group (expandGroups); leave the selection as-is.
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
      try {
        // Rust rotates the clicked face's normal onto -Z (lays it flat).
        await invoke("scene_object_lay_flat_on", {
          ids,
          normal: hit.normal,
          contact: hit.point,
          expandGroups,
        });
      } catch (e) {
        console.error("lay flat failed", e);
      }
      return true;
    };

    // Camera eye position + forward (eye→center) unit vector from the orbit
    // params. The single source for every place that needs the camera basis.
    const camFrame = (): { eye: Vec3; fwd: Vec3 } => {
      const c = cam.current;
      const ce = Math.cos(c.el),
        se = Math.sin(c.el);
      const ca = Math.cos(c.az),
        sa = Math.sin(c.az);
      return {
        eye: [c.center[0] + c.dist * ce * ca, c.center[1] + c.dist * ce * sa, c.center[2] + c.dist * se],
        fwd: [-ce * ca, -ce * sa, -se],
      };
    };

    // Hit-test the cursor for a press or hover — all gizmo math is Rust-side. The
    // result is the active gizmo's handles, else the selected body / empty space.
    const camArgs = () => {
      const c = cam.current;
      const cv = canvasRef.current!;
      return { width: cv.width, height: cv.height, az: c.az, el: c.el, dist: c.dist, center: c.center };
    };
    const grabAt = async (sx: number, sy: number): Promise<GrabResult | null> => {
      if (!canvasRef.current) return null;
      try {
        return await invoke<GrabResult>("viewport_grab", {
          req: { ...camArgs(), x: sx, y: sy, gizmo: gizmoModeRef.current },
        });
      } catch (e) {
        console.error("viewport_grab failed", e);
        return null;
      }
    };
    // Cursor ray ∩ a world plane (Rust unprojects) — the tower drag's only ray need.
    const rayPlaneHit = async (sx: number, sy: number, n: Vec3, p: Vec3): Promise<Vec3 | null> => {
      if (!canvasRef.current) return null;
      try {
        return await invoke<Vec3 | null>("viewport_ray_plane", {
          req: { ...camArgs(), x: sx, y: sy, plane_n: n, plane_p: p },
        });
      } catch (e) {
        console.error("viewport_ray_plane failed", e);
        return null;
      }
    };

    // Idle handle highlight: ask Rust what the cursor would grab, light its handle
    // index. Coalesced (one grab in flight, latest cursor) so fast moves don't
    // flood the IPC channel.
    let hoverInflight = false;
    let hoverPending: [number, number] | null = null;
    const updateHover = (e: MouseEvent) => {
      hoverPending = rel(e);
      if (hoverInflight) return;
      hoverInflight = true;
      void (async () => {
        while (hoverPending) {
          const [sx, sy] = hoverPending;
          hoverPending = null;
          const r = await grabAt(sx, sy);
          const idx = r && r.kind === "gizmo" ? r.grab.idx : -1;
          if (idx !== hoverHandle.current) {
            hoverHandle.current = idx;
            void render();
          }
        }
        hoverInflight = false;
      })();
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
      const [fx, fy, fz] = camFrame().fwd;
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
      // Left: decide what was grabbed via Rust (async). Until it resolves the
      // press stays "pending" and movement is buffered.
      const press: DragState = { x: e.clientX, y: e.clientY, button: 0, mode: "pending" };
      drag.current = press;
      const [sx, sy] = rel(e);

      if (toolRef.current !== "none") {
        // A placing tool is armed: drags orbit, the click (in onUp) runs the tool.
        press.mode = "orbit";
        return;
      }

      void grabAt(sx, sy).then(async (r) => {
        if (drag.current !== press || !r) return;
        if (r.kind === "gizmo") {
          // A handle — or, with no gizmo, the selected body (free-move on its XY
          // plane). The renderer resolves the preview from grab + cursor.
          press.mode = "gizmo";
          press.grab = r.grab;
          press.sx = sx;
          press.sy = sy;
          press.shift = e.shiftKey;
          hoverHandle.current = r.grab.idx; // keep it lit through the drag
        } else if (r.kind === "inert") {
          press.mode = "inert"; // pressed the selected body in a gizmo mode
        } else if (r.kind === "empty") {
          // Empty space: drag the tower if the press hit it, else orbit. The
          // grab + footprint hit-test are Rust-side (viewport_tower_grab returns
          // the grab offset, or null when the press missed / there's no tower).
          const bedHit = await rayPlaneHit(sx, sy, [0, 0, 1], [0, 0, bedMin[2]]);
          if (drag.current !== press) return;
          const offset = bedHit
            ? await invoke<[number, number] | null>("viewport_tower_grab", {
                bx: bedHit[0],
                by: bedHit[1],
              }).catch(() => null)
            : null;
          if (drag.current !== press) return;
          if (offset) {
            press.mode = "tower";
            press.towerOffset = offset;
          } else {
            press.mode = "orbit";
          }
        } else {
          press.mode = "orbit"; // unselected object → orbit (select on click)
        }
      });
    };
    const onUp = (e: MouseEvent) => {
      const d = drag.current;
      drag.current = null;
      if (!d || d.button !== 0) return;
      if (d.mode === "gizmo" && d.grab) {
        // Commit the drag: Rust recomputes the final transform from the release
        // cursor and returns pre·start per selected object. The preview never
        // jumps — the committed pose equals the last previewed one.
        const [sx, sy] = rel(e);
        void invoke<{ id: number; transform: number[] }[]>("viewport_gizmo_commit", {
          req: { ...camArgs(), sx, sy, grab: d.grab, shift: e.shiftKey },
        })
          .then((ups) => {
            if (ups?.length) commitMove(ups.map((u) => ({ id: u.id, m: u.transform })));
          })
          .catch((err) => console.error("gizmo commit failed", err));
        return;
      }
      if (d.mode === "tower") {
        // Commit the Rust-clamped corner as project overrides; the renderer
        // re-resolves to it via project_overrides_changed → invalidate. A click
        // without a drag doesn't pin an override.
        if (d.towerMoved && d.towerClamped && activePlateIdRef.current != null) {
          const plateId = activePlateIdRef.current;
          const [cx, cy] = d.towerClamped;
          void invoke("scene_project_override_set", {
            plateId,
            key: "wipe_tower_x",
            value: fmtCoord(cx),
          }).catch((err) => console.error("override set failed", err));
          void invoke("scene_project_override_set", {
            plateId,
            key: "wipe_tower_y",
            value: fmtCoord(cy),
          }).catch((err) => console.error("override set failed", err));
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
        c.el = Math.min(EL_LIMIT, Math.max(-EL_LIMIT, c.el + dy * 0.01));
        void render();
      } else if (d.mode === "tower" && d.towerOffset) {
        const [sx, sy] = rel(e);
        const off = d.towerOffset;
        void rayPlaneHit(sx, sy, [0, 0, 1], [0, 0, bedMin[2]]).then((bedHit) => {
          if (!bedHit || drag.current !== d) return;
          // Rust clamps the corner to the bed, stores it as the live drag
          // position, and returns it (move-only, no mesh re-upload — smooth drag).
          void invoke<[number, number] | null>("viewport_move_tower", {
            x: bedHit[0] - off[0],
            y: bedHit[1] - off[1],
          }).then((c) => {
            if (!c || drag.current !== d) return;
            d.towerMoved = true;
            d.towerClamped = c;
            void render();
          });
        });
      } else if (d.mode === "gizmo") {
        // Preview-only: stash the latest cursor + Shift; the frame resolves the
        // transform Rust-side via `gizmo_drag`.
        const [sx, sy] = rel(e);
        d.sx = sx;
        d.sy = sy;
        d.shift = e.shiftKey;
        void render();
      }
      // mode === "pending": waiting on the grab — next move acts.
    };
    const onCtxMenu = (e: MouseEvent) => e.preventDefault(); // right-drag pans, no menu
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const c = cam.current;
      c.dist = Math.min(2000, Math.max(20, c.dist * (1 + Math.sign(e.deltaY) * 0.1)));
      void render();
    };

    // Frame the active plate's footprint with a view-aware fit (ports
    // cameraControls.ts frameBox/initialFrameForBed): center on the plate plane,
    // pull the camera back along the current view direction just far enough that
    // every projected corner stays inside the frustum. Z is collapsed to the
    // plate plane so it frames the footprint, not the whole build volume.
    async function reframe() {
      try {
        const info = await invoke<{ min: Vec3; max: Vec3 }>("viewport_scene_info");
        bedMin = info.min;
        const [minX, minY, minZ] = info.min;
        const [maxX, maxY] = info.max;
        const center: Vec3 = [(minX + maxX) / 2, (minY + maxY) / 2, minZ];
        const forward = camFrame().fwd; // eye → center
        const dir = scale(forward, -1); // center → eye
        let right = cross([0, 0, 1], dir);
        right = vlen(right) < 1e-6 ? [1, 0, 0] : norm(right);
        const up = norm(cross(dir, right));
        const cv = canvasRef.current;
        const aspect = cv && cv.clientHeight > 0 ? cv.clientWidth / cv.clientHeight : 1;
        const margin = 1.08;
        const tanV = Math.tan(((45 * Math.PI) / 180) * 0.5);
        const tanH = tanV * aspect;
        let dist = 0.1;
        for (const cx of [minX, maxX]) {
          for (const cy of [minY, maxY]) {
            const v = sub([cx, cy, minZ], center);
            const a = Math.abs(dot(v, right));
            const b = Math.abs(dot(v, up));
            const cc = dot(v, forward);
            dist = Math.max(dist, (a * margin) / tanH - cc, (b * margin) / tanV - cc);
          }
        }
        cam.current.center = center;
        cam.current.dist = dist;
      } catch (e) {
        console.error("viewport_scene_info failed", e);
      }
      void render();
    }

    // A tower-affecting edit (overrides, materials, printer, bed, project load)
    // landed: drop the renderer's cached resolved placement so the next frame
    // re-resolves. A plain plate switch does NOT call this — the cache is keyed
    // per plate and `frame` reads the active plate from the project, so the tower
    // draws in the same frame as the objects (instant).
    const invalidateTower = () => {
      void invoke("viewport_invalidate_tower").then(() => void render());
    };

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

    // Object / material edits change whether (and how big) the tower is →
    // invalidate its resolved placement, then render.
    const offRender = onEvents(
      [
        "scene:mesh_loaded",
        "scene:object_added",
        "scene:object_updated",
        "scene:object_removed",
        "scene:material_slot_changed",
        // Quality profile carries enable_prime_tower / prime_tower_width /
        // wipe_tower_x/y — switching it resizes, moves, or toggles the tower.
        "scene:plate_metadata_changed",
        // Undo/redo swaps the live project: redraw from the restored state and
        // re-resolve the tower, but in place — keep the user's camera (unlike
        // project:loaded, which reframes for fresh geometry).
        "project:restored",
      ],
      () => {
        invalidateTower();
      },
    );
    // Selection doesn't affect the tower → just redraw (no re-resolve).
    const offSelection = onEvents(["scene:selection_changed"], () => void render());
    // A plate switch just renders — `frame` resolves the new plate's tower (the
    // cache is per plate) so it draws in the same frame as the objects. A bed
    // change can alter the cascade-resolved placement → invalidate. (No more
    // lagging-ref problem: the active plate comes from the project, Rust-side.)
    const offActivePlate = onEvents(["scene:active_plate_changed"], () => void reframe());
    const offBed = onEvents(["scene:bed_changed"], () => {
      invalidateTower();
      void reframe();
    });
    // A committed tower drag (or any project override) changes the resolved
    // placement → invalidate so it re-resolves to the clamped position.
    const offOverrides = onEvents(["scene:project_overrides_changed"], () => invalidateTower());
    // A new project replaced the scene: the renderer's caches (meshes + tower)
    // were dropped Rust-side by the replace command (see `project_io`); reframe
    // for the fresh geometry (which renders → resolves the new active tower).
    const offLoaded = onEvents(["project:loaded"], () => void reframe());
    // A slice stores the active plate's tower mesh Rust-side (the slice event
    // sink) before this event arrives; just render so the box flips to the mesh
    // (placement is unchanged by slicing).
    const offTower = onEvents<{ data?: { plate_id?: number } }>(
      ["slice:plate_finished"],
      (e) => {
        if (e.payload?.data?.plate_id === activePlateIdRef.current) void render();
      },
    );

    // Print thumbnail for the send/export path: Rust renders an iso view of the
    // models only (transparent bg) to RGBA; encode it to a PNG via a canvas.
    registerThumbnailCapture(async (size = 512) => {
      try {
        const buf = await invoke<ArrayBuffer>("viewport_thumbnail", { size });
        const tc = document.createElement("canvas");
        tc.width = size;
        tc.height = size;
        const tctx = tc.getContext("2d");
        if (!tctx) return null;
        tctx.putImageData(new ImageData(new Uint8ClampedArray(buf), size, size), 0, 0);
        const url = tc.toDataURL("image/png");
        const prefix = "data:image/png;base64,";
        return url.startsWith(prefix) ? url.slice(prefix.length) : null;
      } catch (e) {
        console.error("viewport_thumbnail failed", e);
        return null;
      }
    });

    // Axis-snap views from the legend's X/Y/Z chips. X → front (look along +Y,
    // X horizontal), Y → side (look along -X, Y horizontal), Z → top (straight
    // down). Reorients only; keeps the current center + zoom.
    registerAxisView((axis: AxisView) => {
      const c = cam.current;
      if (axis === "x") {
        c.az = -Math.PI / 2;
        c.el = 0;
      } else if (axis === "y") {
        c.az = 0;
        c.el = 0;
      } else {
        c.el = EL_LIMIT; // top-down (the same near-pole limit orbit clamps to)
      }
      void render();
    });

    renderRef.current = render;
    void reframe(); // renders → resolves the active plate's tower in-frame

    return () => {
      renderRef.current = null;
      offRender();
      offSelection();
      offActivePlate();
      offBed();
      offOverrides();
      offLoaded();
      offTower();
      registerThumbnailCapture(null);
      registerAxisView(null);
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

  // Toggling gizmo mode: reset the hovered handle, redraw (the renderer draws the
  // gizmo for the new mode from the selection).
  useEffect(() => {
    hoverHandle.current = -1;
    renderRef.current?.();
  }, [gizmoMode]);

  // Leaving face-match (cancel / switch / complete) drops the stashed reference.
  useEffect(() => {
    if (tool !== "facematch") faceMatchRef.current = null;
  }, [tool]);

  return <canvas ref={canvasRef} style={{ width: "100%", height: "100%", display: "block" }} />;
}
