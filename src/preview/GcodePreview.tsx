// G-code toolpath preview — Strategy-A wgpu driver.
//
// The toolpath renders in Rust (wgpu, offscreen, instanced tubes) and
// the finished frame is blitted into this opaque `<canvas>`. This
// component is a thin driver: it owns the orbit camera (az/el/dist/
// center) frontend-side, captures navigation input, and calls
// `toolpath_frame` → `putImageData` on demand (coalesced, one in
// flight). Geometry never crosses the IPC bridge — it's GPU-resident,
// keyed by the preview handle. Mirrors `viewport/WgpuViewport.tsx`.
//
// Pure props: layer window, color mode, visibility toggles all live in
// PreviewWorkspace and flow down; the driver just reflects them.

import { useEffect, useRef } from "react";

import { ViewportLegend } from "../viewport/ViewportLegend";
import { registerAxisView, setAxisView, type AxisView } from "../viewport/cameraControl";

import { toolpathFrame, toolpathPick, previewSegmentDetail } from "./invokes";
import { windowBounds } from "./layerWindow";
import type {
  BoundingBox,
  ColorMode,
  LayerWindow,
  Palette,
  PreviewLoadResponse,
  SegmentDetail,
} from "./types";

type Vec3 = [number, number, number];
// Orbit elevation limit — just shy of ±90° so world-Z up never degenerates.
const EL_LIMIT = Math.PI / 2 - 0.001;
const sub = (a: Vec3, b: Vec3): Vec3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
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

export interface GcodePreviewProps {
  /** Loaded preview handle + counts. `null` renders an empty canvas. */
  preview: PreviewLoadResponse | null;
  /** Bed extents — used to frame the view when the print bbox is absent. */
  bedExtents: BoundingBox | null;
  colorMode: ColorMode;
  palette: Palette;
  layerWindow: LayerWindow;
  showTravels: boolean;
  showRetractions: boolean;
  /** Hover-inspection callback. `null` clears the hover state. */
  onSegmentHover?: (detail: SegmentDetail | null) => void;
}

export function GcodePreview({
  preview,
  bedExtents,
  colorMode,
  palette,
  layerWindow,
  showTravels,
  showRetractions,
  onSegmentHover,
}: GcodePreviewProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  // Default framing matches the prepare viewport: az = -90° (X axis along the
  // bottom), ~37° elevation. reframe() refits to the print on load.
  const cam = useRef({
    az: -Math.PI / 2,
    el: Math.atan2(200, 260),
    dist: 350,
    center: [0, 0, 0] as Vec3,
  });

  // Kept fresh each render so the mount-once effect's handlers read live props.
  const previewRef = useRef(preview);
  previewRef.current = preview;
  const colorModeRef = useRef(colorMode);
  colorModeRef.current = colorMode;
  const paletteRef = useRef(palette);
  paletteRef.current = palette;
  const layerWindowRef = useRef(layerWindow);
  layerWindowRef.current = layerWindow;
  const showTravelsRef = useRef(showTravels);
  showTravelsRef.current = showTravels;
  const showRetractionsRef = useRef(showRetractions);
  showRetractionsRef.current = showRetractions;
  const onHoverRef = useRef(onSegmentHover);
  onHoverRef.current = onSegmentHover;
  const bedExtentsRef = useRef(bedExtents);
  bedExtentsRef.current = bedExtents;
  // Lets prop-change effects outside the mount-once effect trigger a redraw /
  // refit the camera.
  const renderRef = useRef<(() => void) | null>(null);
  const reframeRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const inflight = { busy: false, dirty: false };
    async function render() {
      if (inflight.busy) {
        inflight.dirty = true;
        return;
      }
      const cv = canvasRef.current;
      if (!cv) return;
      const w = Math.max(1, cv.clientWidth);
      const h = Math.max(1, cv.clientHeight);
      if (cv.width !== w) cv.width = w;
      if (cv.height !== h) cv.height = h;
      const p = previewRef.current;
      if (!p) {
        // No preview loaded → clear to the renderer's background.
        ctx!.fillStyle = "#1a1a1f";
        ctx!.fillRect(0, 0, w, h);
        return;
      }
      inflight.busy = true;
      try {
        const c = cam.current;
        const [lmin, lmax] = windowBounds(layerWindowRef.current);
        const buf = await toolpathFrame({
          handle: p.handle,
          width: w,
          height: h,
          az: c.az,
          el: c.el,
          dist: c.dist,
          center: c.center,
          layer_min: lmin,
          layer_max: lmax,
          color_mode: colorModeRef.current,
          palette: paletteRef.current,
          show_travels: showTravelsRef.current,
          show_retractions: showRetractionsRef.current,
          bed_min: bedExtentsRef.current?.min ?? null,
          bed_max: bedExtentsRef.current?.max ?? null,
        });
        ctx!.putImageData(new ImageData(new Uint8ClampedArray(buf), w, h), 0, 0);
      } catch (e) {
        console.error("toolpath_frame failed", e);
      } finally {
        inflight.busy = false;
        if (inflight.dirty) {
          inflight.dirty = false;
          void render();
        }
      }
    }
    renderRef.current = render;

    const rel = (e: MouseEvent): [number, number] => {
      const r = canvas.getBoundingClientRect();
      return [e.clientX - r.left, e.clientY - r.top];
    };
    const camFrame = (): { fwd: Vec3 } => {
      const c = cam.current;
      const ce = Math.cos(c.el),
        se = Math.sin(c.el);
      const ca = Math.cos(c.az),
        sa = Math.sin(c.az);
      return { fwd: [-ce * ca, -ce * sa, -se] };
    };
    // Right-drag pan: shift the orbit center so the grabbed point tracks the
    // cursor (world-units-per-pixel at the focal plane). Ports WgpuViewport.
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

    type Drag = { x: number; y: number; mode: "orbit" | "pan" };
    let drag: Drag | null = null;
    const onDown = (e: MouseEvent) => {
      if (e.button !== 0 && e.button !== 2) return;
      if (e.button === 2) e.preventDefault();
      drag = { x: e.clientX, y: e.clientY, mode: e.button === 2 ? "pan" : "orbit" };
    };
    const onUp = () => {
      drag = null;
    };
    const onMove = (e: MouseEvent) => {
      if (!drag) {
        updateHover(e);
        return;
      }
      const dx = e.clientX - drag.x;
      const dy = e.clientY - drag.y;
      drag.x = e.clientX;
      drag.y = e.clientY;
      if (drag.mode === "pan") {
        pan(dx, dy);
      } else {
        const c = cam.current;
        c.az -= dx * 0.01;
        c.el = Math.min(EL_LIMIT, Math.max(-EL_LIMIT, c.el + dy * 0.01));
        void render();
      }
    };
    const onCtxMenu = (e: MouseEvent) => e.preventDefault();
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const c = cam.current;
      c.dist = Math.min(2000, Math.max(20, c.dist * (1 + Math.sign(e.deltaY) * 0.1)));
      void render();
    };

    // Hover inspection: Rust picks the nearest visible segment, then we fetch
    // its detail. Coalesced (one pick in flight, latest cursor) so a fast
    // cursor doesn't flood the IPC channel. Skipped without a hover callback.
    let hoverInflight = false;
    let hoverPending: [number, number] | null = null;
    let lastSegment: number | null = null;
    const emitHover = (d: SegmentDetail | null) => onHoverRef.current?.(d);
    const updateHover = (e: MouseEvent) => {
      if (!onHoverRef.current || !previewRef.current) return;
      hoverPending = rel(e);
      if (hoverInflight) return;
      hoverInflight = true;
      void (async () => {
        while (hoverPending) {
          const [sx, sy] = hoverPending;
          hoverPending = null;
          const p = previewRef.current;
          const cv = canvasRef.current;
          if (!p || !cv) break;
          const c = cam.current;
          const [lmin, lmax] = windowBounds(layerWindowRef.current);
          try {
            const seg = await toolpathPick({
              handle: p.handle,
              width: cv.width,
              height: cv.height,
              x: sx,
              y: sy,
              az: c.az,
              el: c.el,
              dist: c.dist,
              center: c.center,
              layer_min: lmin,
              layer_max: lmax,
            });
            if (seg == null) {
              if (lastSegment != null) {
                lastSegment = null;
                emitHover(null);
              }
              continue;
            }
            if (seg === lastSegment) continue;
            lastSegment = seg;
            const detail = await previewSegmentDetail(p.handle, seg);
            if (lastSegment === seg) emitHover(detail);
          } catch (err) {
            console.error("toolpath_pick failed", err);
          }
        }
        hoverInflight = false;
      })();
    };
    const onLeave = () => {
      if (lastSegment != null) {
        lastSegment = null;
        emitHover(null);
      }
    };

    // View-aware fit: center on the print (or bed) and pull the camera back
    // along the current view direction just far enough that every bbox corner
    // stays in frame. Ports WgpuViewport.reframe / cameraControls.frameBox.
    function reframe() {
      const p = previewRef.current;
      const bbox = p?.bounding_box ?? bedExtentsRef.current;
      if (!bbox) {
        void render();
        return;
      }
      const [minX, minY, minZ] = bbox.min;
      const [maxX, maxY, maxZ] = bbox.max;
      const center: Vec3 = [(minX + maxX) / 2, (minY + maxY) / 2, (minZ + maxZ) / 2];
      const forward = camFrame().fwd;
      const dir: Vec3 = [-forward[0], -forward[1], -forward[2]]; // center → eye
      let right = cross([0, 0, 1], dir);
      right = vlen(right) < 1e-6 ? [1, 0, 0] : norm(right);
      const up = norm(cross(dir, right));
      const cv = canvasRef.current;
      const aspect = cv && cv.clientHeight > 0 ? cv.clientWidth / cv.clientHeight : 1;
      const margin = 1.1;
      const tanV = Math.tan(((45 * Math.PI) / 180) * 0.5);
      const tanH = tanV * aspect;
      let dist = 20;
      for (const cx of [minX, maxX]) {
        for (const cy of [minY, maxY]) {
          for (const cz of [minZ, maxZ]) {
            const v = sub([cx, cy, cz], center);
            const a = Math.abs(dot(v, right));
            const b = Math.abs(dot(v, up));
            const cc = dot(v, forward);
            dist = Math.max(dist, (a * margin) / tanH - cc, (b * margin) / tanV - cc);
          }
        }
      }
      cam.current.center = center;
      cam.current.dist = dist;
      void render();
    }
    reframeRef.current = reframe;

    // Axis-snap views from the legend's X/Y/Z chips. X → front (look along +Y),
    // Y → side (look along -X), Z → top. Reorients only; keeps center + zoom.
    registerAxisView((axis: AxisView) => {
      const c = cam.current;
      if (axis === "x") {
        c.az = -Math.PI / 2;
        c.el = 0;
      } else if (axis === "y") {
        c.az = 0;
        c.el = 0;
      } else {
        c.el = EL_LIMIT;
      }
      void render();
    });

    const ro = new ResizeObserver(() => void render());
    ro.observe(canvas);
    canvas.addEventListener("mousedown", onDown);
    window.addEventListener("mouseup", onUp);
    window.addEventListener("mousemove", onMove);
    canvas.addEventListener("mouseleave", onLeave);
    canvas.addEventListener("wheel", onWheel, { passive: false });
    canvas.addEventListener("contextmenu", onCtxMenu);

    reframe();

    return () => {
      renderRef.current = null;
      reframeRef.current = null;
      registerAxisView(null);
      ro.disconnect();
      canvas.removeEventListener("mousedown", onDown);
      window.removeEventListener("mouseup", onUp);
      window.removeEventListener("mousemove", onMove);
      canvas.removeEventListener("mouseleave", onLeave);
      canvas.removeEventListener("wheel", onWheel);
      canvas.removeEventListener("contextmenu", onCtxMenu);
    };
    // Mount-once: handlers read live props via refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Refit the camera only when a new print loads — NOT on every re-render.
  // (PreviewWorkspace re-renders on each mousemove; keying on the bedExtents
  // object identity would snap the camera back mid-zoom.)
  useEffect(() => {
    reframeRef.current?.();
  }, [preview?.handle]);

  // Any other prop that changes the rendered picture → redraw.
  useEffect(() => {
    renderRef.current?.();
  }, [colorMode, palette, layerWindow, showTravels, showRetractions]);

  return (
    <div style={{ width: "100%", height: "100%", position: "relative" }}>
      <canvas ref={canvasRef} style={{ width: "100%", height: "100%", display: "block" }} />
      <ViewportLegend hints="LMB rotate · RMB pan · scroll zoom" onAxis={setAxisView} />
    </div>
  );
}
