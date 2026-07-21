// Demo replacement for the native (Rust wgpu) prepare viewport. Renders the
// sample model — the OrangeCon M5 StickS3 case — as a lit solid on a build
// plate, orbitable, in the browser. Accepts WgpuViewport's props but ignores
// the editing tools (inert in the demo).
import { useEffect, useRef } from "react";
import mesh from "../assets/mesh.json";
import { attachOrbit, lookAt, mul, perspective, type Orbit } from "./glcam";

const BED = 180; // A1 mini build plate (mm), 10 mm grid.

const MESH_VS = `
attribute vec3 aPos, aNormal;
uniform mat4 uVP;
varying vec3 vN, vP;
void main() { vN = aNormal; vP = aPos; gl_Position = uVP * vec4(aPos, 1.0); }`;
const MESH_FS = `
precision mediump float;
varying vec3 vN, vP;
uniform vec3 uColor, uEye;
void main() {
  vec3 n = normalize(vN);
  vec3 l = normalize(vec3(0.35, 0.5, 1.0));
  float diff = max(dot(n, l), 0.0);
  vec3 v = normalize(uEye - vP);
  float rim = pow(1.0 - max(dot(n, v), 0.0), 2.5) * 0.35;
  vec3 c = uColor * (0.28 + 0.72 * diff) + rim * vec3(0.4, 0.55, 0.7);
  gl_FragColor = vec4(c, 1.0);
}`;
const LINE_VS = `attribute vec3 aPos; uniform mat4 uVP; void main() { gl_Position = uVP * vec4(aPos, 1.0); }`;
const LINE_FS = `precision mediump float; uniform vec3 uColor; void main() { gl_FragColor = vec4(uColor, 1.0); }`;

function computeNormals(pos: number[], idx: number[]): Float32Array {
  const n = new Float32Array(pos.length);
  for (let i = 0; i < idx.length; i += 3) {
    const a = idx[i] * 3, b = idx[i + 1] * 3, c = idx[i + 2] * 3;
    const ux = pos[b] - pos[a], uy = pos[b + 1] - pos[a + 1], uz = pos[b + 2] - pos[a + 2];
    const vx = pos[c] - pos[a], vy = pos[c + 1] - pos[a + 1], vz = pos[c + 2] - pos[a + 2];
    const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
    for (const o of [a, b, c]) { n[o] += nx; n[o + 1] += ny; n[o + 2] += nz; }
  }
  for (let i = 0; i < n.length; i += 3) {
    const l = Math.hypot(n[i], n[i + 1], n[i + 2]) || 1;
    n[i] /= l; n[i + 1] /= l; n[i + 2] /= l;
  }
  return n;
}

// Grid lines (inner) + border (outer square) on z=0, centered at (cx, cy).
function bedLines(cx: number, cy: number): { grid: Float32Array; border: Float32Array } {
  const h = BED / 2, grid: number[] = [], border: number[] = [];
  for (let i = -h; i <= h; i += 10) {
    const isEdge = i === -h || i === h;
    const t = isEdge ? border : grid;
    t.push(cx + i, cy - h, 0, cx + i, cy + h, 0); // vertical line
    t.push(cx - h, cy + i, 0, cx + h, cy + i, 0); // horizontal line
  }
  return { grid: new Float32Array(grid), border: new Float32Array(border) };
}

export function WgpuViewport(_props: Record<string, unknown>): React.JSX.Element {
  const ref = useRef<HTMLCanvasElement | null>(null);
  useEffect(() => {
    const canvas = ref.current!;
    const gl = canvas.getContext("webgl", { antialias: true })!;
    const pos = mesh.positions as number[];
    const idx = mesh.indices as number[];
    const link = (vs: string, fs: string) => {
      const p = gl.createProgram()!;
      const mk = (t: number, s: string) => { const sh = gl.createShader(t)!; gl.shaderSource(sh, s); gl.compileShader(sh); return sh; };
      gl.attachShader(p, mk(gl.VERTEX_SHADER, vs)); gl.attachShader(p, mk(gl.FRAGMENT_SHADER, fs)); gl.linkProgram(p);
      return p;
    };
    const meshProg = link(MESH_VS, MESH_FS);
    const lineProg = link(LINE_VS, LINE_FS);

    const buf = (arr: BufferSource) => { const b = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, b); gl.bufferData(gl.ARRAY_BUFFER, arr, gl.STATIC_DRAW); return b; };
    const posBuf = buf(new Float32Array(pos));
    const nrmBuf = buf(computeNormals(pos, idx));
    const idxBuf = gl.createBuffer()!;
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, idxBuf);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, new Uint32Array(idx), gl.STATIC_DRAW);
    const idxType = gl.getExtension("OES_element_index_uint") ? gl.UNSIGNED_INT : gl.UNSIGNED_SHORT;

    gl.enable(gl.DEPTH_TEST);
    gl.clearColor(0.055, 0.07, 0.09, 1);

    // model bbox
    const lo = [1e9, 1e9, 1e9], hi = [-1e9, -1e9, -1e9];
    for (let i = 0; i < pos.length; i += 3) for (let k = 0; k < 3; k++) { lo[k] = Math.min(lo[k], pos[i + k]); hi[k] = Math.max(hi[k], pos[i + k]); }
    const cx = (lo[0] + hi[0]) / 2, cy = (lo[1] + hi[1]) / 2, cz = (lo[2] + hi[2]) / 2;
    const { grid, border } = bedLines(cx, cy);
    const gridBuf = buf(grid), borderBuf = buf(border);

    // Orbit around the model, framed to show the whole build plate around it.
    const target = [cx, cy, cz];
    const o: Orbit = { az: 0.6, el: 0.5, dist: BED * 1.15, autoRotate: false };
    const detach = attachOrbit(canvas, o, BED * 0.35, BED * 3);

    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const resize = () => { canvas.width = Math.floor(canvas.clientWidth * dpr); canvas.height = Math.floor(canvas.clientHeight * dpr); };
    resize();
    const roz = new ResizeObserver(resize); roz.observe(canvas);

    const drawLines = (b: WebGLBuffer, n: number, color: [number, number, number], vp: number[]) => {
      gl.useProgram(lineProg);
      const l = gl.getAttribLocation(lineProg, "aPos");
      gl.bindBuffer(gl.ARRAY_BUFFER, b); gl.enableVertexAttribArray(l); gl.vertexAttribPointer(l, 3, gl.FLOAT, false, 0, 0);
      gl.uniformMatrix4fv(gl.getUniformLocation(lineProg, "uVP"), false, vp);
      gl.uniform3f(gl.getUniformLocation(lineProg, "uColor"), color[0], color[1], color[2]);
      gl.drawArrays(gl.LINES, 0, n / 3);
    };

    let raf = 0;
    const frame = () => {
      const eye = [
        target[0] + o.dist * Math.cos(o.el) * Math.sin(o.az),
        target[1] + o.dist * Math.cos(o.el) * Math.cos(o.az),
        target[2] + o.dist * Math.sin(o.el),
      ];
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      const aspect = canvas.width / Math.max(canvas.height, 1);
      const vp = mul(perspective(0.8, aspect, BED * 0.02, BED * 8), lookAt(eye, target, [0, 0, 1]));

      drawLines(gridBuf, grid.length, [0.16, 0.21, 0.26], vp);
      drawLines(borderBuf, border.length, [0.3, 0.4, 0.48], vp);

      gl.useProgram(meshProg);
      const pl = gl.getAttribLocation(meshProg, "aPos");
      gl.bindBuffer(gl.ARRAY_BUFFER, posBuf); gl.enableVertexAttribArray(pl); gl.vertexAttribPointer(pl, 3, gl.FLOAT, false, 0, 0);
      const nl = gl.getAttribLocation(meshProg, "aNormal");
      gl.bindBuffer(gl.ARRAY_BUFFER, nrmBuf); gl.enableVertexAttribArray(nl); gl.vertexAttribPointer(nl, 3, gl.FLOAT, false, 0, 0);
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, idxBuf);
      gl.uniformMatrix4fv(gl.getUniformLocation(meshProg, "uVP"), false, vp);
      gl.uniform3f(gl.getUniformLocation(meshProg, "uColor"), 0.93, 0.44, 0.16); // OrangeCon orange
      gl.uniform3f(gl.getUniformLocation(meshProg, "uEye"), eye[0], eye[1], eye[2]);
      gl.drawElements(gl.TRIANGLES, idx.length, idxType, 0);

      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => { cancelAnimationFrame(raf); detach(); roz.disconnect(); };
  }, []);
  return <canvas ref={ref} style={{ position: "absolute", inset: 0, width: "100%", height: "100%", display: "block", background: "#0e1217" }} />;
}
