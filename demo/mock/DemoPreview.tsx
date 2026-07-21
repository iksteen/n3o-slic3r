// Demo replacement for the native gcode preview (GcodePreview). Renders the
// canned toolpaths (from a real n3o slice of the sample model) as feature-
// colored fat-line tubes, with a print-build animation. Accepts GcodePreviewProps
// but drives itself (the demo shows one fixed sliced result).
import { useEffect, useRef } from "react";
import toolpaths from "../assets/toolpaths.json";
import { attachOrbit, lookAt, mul, perspective, type Orbit } from "./glcam";

const FEATURE_COLORS: Record<string, [number, number, number]> = {
  "Outer wall": [1.0, 0.36, 0.24], "Inner wall": [0.24, 0.82, 0.44],
  "Overhang wall": [0.2, 0.8, 0.85], "Sparse infill": [0.85, 0.63, 0.27],
  "Internal solid infill": [0.9, 0.78, 0.28], "Gap infill": [0.95, 0.42, 0.72],
  "Top surface": [0.32, 0.66, 1.0], "Bottom surface": [0.2, 0.52, 0.86],
  "Internal Bridge": [0.7, 0.45, 1.0],
};
const FALLBACK: [number, number, number] = [0.6, 0.66, 0.72];

const VS = `
attribute vec2 aCorner; attribute vec3 aStart, aEnd, aColor; attribute float aLayer;
uniform mat4 uVP; uniform vec2 uViewport; uniform float uWidth, uMaxLayer, uMinLayer;
varying vec3 vColor; varying float vSide;
void main() {
  if (aLayer > uMaxLayer || aLayer < uMinLayer) { gl_Position = vec4(2.0,2.0,2.0,1.0); return; }
  vec4 cs = uVP * vec4(aStart,1.0), ce = uVP * vec4(aEnd,1.0);
  vec4 clip = mix(cs, ce, aCorner.x);
  vec2 s = cs.xy/cs.w, e = ce.xy/ce.w;
  vec2 dir = normalize((e-s)*uViewport + vec2(1e-6,0.0));
  clip.xy += vec2(-dir.y, dir.x)/uViewport * aCorner.y * uWidth * clip.w;
  gl_Position = clip; vColor = aColor; vSide = aCorner.y;
}`;
const FS = `
precision mediump float; varying vec3 vColor; varying float vSide;
void main() { float a = 1.0-abs(vSide); gl_FragColor = vec4(vColor*(0.5+0.5*sqrt(max(a,0.0))), 1.0); }`;

type LayerWindow =
  | { mode: "single"; layer: number }
  | { mode: "up-to"; max: number }
  | { mode: "range"; min: number; max: number };

export function GcodePreview(props: { layerWindow?: LayerWindow }): React.JSX.Element {
  const ref = useRef<HTMLCanvasElement | null>(null);
  // Kept fresh each render so the mount-once render loop reads the live window.
  const lwRef = useRef<LayerWindow | undefined>(props.layerWindow);
  lwRef.current = props.layerWindow;
  useEffect(() => {
    const canvas = ref.current!;
    const gl = canvas.getContext("webgl", { antialias: true })!;
    const ext = gl.getExtension("ANGLE_instanced_arrays")!;
    const data = toolpaths as { features: string[]; bbox: number[]; layers: { z: number; paths: number[][] }[] };

    const starts: number[] = [], ends: number[] = [], colors: number[] = [], layers: number[] = [];
    data.layers.forEach((ly, li) => {
      for (const p of ly.paths) {
        const col = FEATURE_COLORS[data.features[p[0]]] || FALLBACK;
        for (let i = 1; i + 3 < p.length; i += 2) {
          starts.push(p[i], p[i + 1], ly.z); ends.push(p[i + 2], p[i + 3], ly.z);
          colors.push(col[0], col[1], col[2]); layers.push(li);
        }
      }
    });
    const count = layers.length, N = data.layers.length;

    const p = gl.createProgram()!;
    const mk = (t: number, s: string) => { const sh = gl.createShader(t)!; gl.shaderSource(sh, s); gl.compileShader(sh); return sh; };
    gl.attachShader(p, mk(gl.VERTEX_SHADER, VS)); gl.attachShader(p, mk(gl.FRAGMENT_SHADER, FS));
    gl.linkProgram(p); gl.useProgram(p);
    const buf = (a: BufferSource) => { const b = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, b); gl.bufferData(gl.ARRAY_BUFFER, a, gl.STATIC_DRAW); return b; };
    const bStart = buf(new Float32Array(starts)), bEnd = buf(new Float32Array(ends)), bCol = buf(new Float32Array(colors)), bLayer = buf(new Float32Array(layers));
    const corner = buf(new Float32Array([0, -1, 1, -1, 1, 1, 0, -1, 1, 1, 0, 1]));
    const U = { VP: gl.getUniformLocation(p, "uVP"), vp2: gl.getUniformLocation(p, "uViewport"), w: gl.getUniformLocation(p, "uWidth"), ml: gl.getUniformLocation(p, "uMaxLayer"), mn: gl.getUniformLocation(p, "uMinLayer") };
    gl.enable(gl.DEPTH_TEST); gl.clearColor(0.055, 0.07, 0.09, 1);

    const bb = data.bbox;
    const baseCenter = [(bb[0] + bb[3]) / 2, (bb[1] + bb[4]) / 2, (bb[2] + bb[5]) / 2];
    const radius = Math.hypot(bb[3] - bb[0], bb[4] - bb[1], bb[5] - bb[2]) / 2;
    const o: Orbit = { az: 0.7, el: 0.5, dist: radius * 2.7, autoRotate: false, pan: [0, 0, 0] };
    const detach = attachOrbit(canvas, o, radius * 1.3, radius * 6);

    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const resize = () => { canvas.width = Math.floor(canvas.clientWidth * dpr); canvas.height = Math.floor(canvas.clientHeight * dpr); };
    resize(); const roz = new ResizeObserver(resize); roz.observe(canvas);

    const setI = (name: string, b: WebGLBuffer, size: number) => { const l = gl.getAttribLocation(p, name); gl.bindBuffer(gl.ARRAY_BUFFER, b); gl.enableVertexAttribArray(l); gl.vertexAttribPointer(l, size, gl.FLOAT, false, 0, 0); ext.vertexAttribDivisorANGLE(l, 1); };

    let raf = 0, last = performance.now();
    const frame = (now: number) => {
      const dt = Math.min((now - last) / 1000, 0.05); last = now;
      if (o.autoRotate) o.az += dt * 0.25;
      // Layer window (0-based, matching the data). Default: all layers.
      const lw = lwRef.current;
      let minLayer = 0, maxLayer = N;
      if (lw?.mode === "single") { minLayer = lw.layer; maxLayer = lw.layer; }
      else if (lw?.mode === "up-to") { maxLayer = lw.max; }
      else if (lw?.mode === "range") { minLayer = lw.min; maxLayer = lw.max; }
      const center = [baseCenter[0] + o.pan[0], baseCenter[1] + o.pan[1], baseCenter[2] + o.pan[2]];
      const eye = [center[0] + o.dist * Math.cos(o.el) * Math.sin(o.az), center[1] + o.dist * Math.cos(o.el) * Math.cos(o.az), center[2] + o.dist * Math.sin(o.el)];
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      gl.useProgram(p);
      const aspect = canvas.width / Math.max(canvas.height, 1);
      const vp = mul(perspective(0.9, aspect, radius * 0.05, radius * 20), lookAt(eye, center, [0, 0, 1]));
      const cl = gl.getAttribLocation(p, "aCorner"); gl.bindBuffer(gl.ARRAY_BUFFER, corner); gl.enableVertexAttribArray(cl); gl.vertexAttribPointer(cl, 2, gl.FLOAT, false, 0, 0); ext.vertexAttribDivisorANGLE(cl, 0);
      setI("aStart", bStart, 3); setI("aEnd", bEnd, 3); setI("aColor", bCol, 3); setI("aLayer", bLayer, 1);
      gl.uniformMatrix4fv(U.VP, false, vp); gl.uniform2f(U.vp2, canvas.width, canvas.height); gl.uniform1f(U.w, 3.0); gl.uniform1f(U.ml, maxLayer); gl.uniform1f(U.mn, minLayer);
      ext.drawArraysInstancedANGLE(gl.TRIANGLES, 0, 6, count);
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => { cancelAnimationFrame(raf); detach(); roz.disconnect(); };
  }, []);
  return <canvas ref={ref} style={{ position: "absolute", inset: 0, width: "100%", height: "100%", display: "block", background: "#0e1217" }} />;
}
