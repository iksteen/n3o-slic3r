import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * Strategy-A wgpu viewport (Linux, `N3O_WGPU=1`): the 3D scene is rendered in
 * Rust (wgpu, offscreen) and the finished frame is blitted into this opaque
 * `<canvas>`. WebKitGTK can't composite a transparent webview over GPU content
 * (it smears dynamic DOM — see docs/dev/wgpu-renderer.md), so the render comes
 * to the webview as pixels rather than the webview sitting over a GL surface.
 *
 * On-demand: a frame is requested only on camera/size change. Requests coalesce
 * (one in flight; a change during a render re-renders on completion).
 *
 * Foundation: orbit (left-drag) + zoom (wheel) over a build-plate grid. Real
 * scene meshes + the proper camera framing land next.
 */
export function WgpuViewport() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const cam = useRef({ az: 0.9, el: 0.6, dist: 350, center: [0, 0, 0] as [number, number, number] });
  const drag = useRef<{ x: number; y: number } | null>(null);
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
        const buf = await invoke<ArrayBuffer>("viewport_frame", {
          req: { width: w, height: h, az: c.az, el: c.el, dist: c.dist, center: c.center },
        });
        const img = new ImageData(new Uint8ClampedArray(buf), w, h);
        ctx?.putImageData(img, 0, 0);
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

    const onDown = (e: MouseEvent) => {
      if (e.button === 0) drag.current = { x: e.clientX, y: e.clientY };
    };
    const onUp = () => {
      drag.current = null;
    };
    const onMove = (e: MouseEvent) => {
      if (!drag.current) return;
      const dx = e.clientX - drag.current.x;
      const dy = e.clientY - drag.current.y;
      drag.current = { x: e.clientX, y: e.clientY };
      const c = cam.current;
      c.az -= dx * 0.01;
      c.el = Math.min(1.45, Math.max(-1.45, c.el + dy * 0.01));
      void render();
    };
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const c = cam.current;
      c.dist = Math.min(2000, Math.max(20, c.dist * (1 + Math.sign(e.deltaY) * 0.1)));
      void render();
    };
    const ro = new ResizeObserver(() => void render());

    canvas.addEventListener("mousedown", onDown);
    window.addEventListener("mouseup", onUp);
    window.addEventListener("mousemove", onMove);
    canvas.addEventListener("wheel", onWheel, { passive: false });
    ro.observe(canvas);
    void render();

    return () => {
      canvas.removeEventListener("mousedown", onDown);
      window.removeEventListener("mouseup", onUp);
      window.removeEventListener("mousemove", onMove);
      canvas.removeEventListener("wheel", onWheel);
      ro.disconnect();
    };
  }, []);

  return <canvas ref={canvasRef} style={{ width: "100%", height: "100%", display: "block" }} />;
}
