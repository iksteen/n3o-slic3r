// Demo replacement for the native (Rust wgpu) prepare viewport. Renders the
// sample model — the OrangeCon M5 StickS3 case — as a lit solid on a build
// plate, orbitable, in the browser. Accepts WgpuViewport's props but ignores
// the editing tools (inert in the demo).
import { useEffect, useRef } from "react";
import parts from "../assets/parts.json";
import { attachOrbit, lookAt, mul, perspective, type Orbit } from "./glcam";

const BED = 180; // A1 mini build plate (mm), 10 mm grid.

// Per-material colours, keyed by extruder (1-based). Match the AMS slot colours
// the material chips resolve to: body → slot 0 / A1 (OrangeCon orange), logo
// insert → slot 3 / A4 (black). Any other extruder falls back to grey.
const EXTRUDER_RGB: Record<number, [number, number, number]> = {
  1: [0.93, 0.44, 0.16],
  2: [0.06, 0.06, 0.07],
};
const FALLBACK_RGB: [number, number, number] = [0.6, 0.62, 0.66];

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

// Crease-aware vertex normals (mirrors the Rust viewport's crease_verts). Plain
// smooth normals bleed a fillet's slant across an adjacent flat face, which a
// coarse fan triangulation interpolates into radial triangular artifacts. Faces
// meeting steeper than CREASE are a hard edge: split the shared vertex so each
// smoothing group keeps its own normal (flat stays flat, curves stay smooth).
// Returns split positions + normals + a remapped index buffer.
const CREASE_COS = 0.866; // ~30°
function creaseVerts(pos: number[], idx: number[]): {
  positions: Float32Array; normals: Float32Array; indices: Uint32Array;
} {
  const vcount = pos.length / 3, tcount = idx.length / 3;
  // Area-weighted (for accumulation) + unit (for the angle test) face normals.
  const faceAw = new Float32Array(tcount * 3), faceUnit = new Float32Array(tcount * 3);
  for (let t = 0; t < tcount; t++) {
    const a = idx[3 * t] * 3, b = idx[3 * t + 1] * 3, c = idx[3 * t + 2] * 3;
    const ux = pos[b] - pos[a], uy = pos[b + 1] - pos[a + 1], uz = pos[b + 2] - pos[a + 2];
    const vx = pos[c] - pos[a], vy = pos[c + 1] - pos[a + 1], vz = pos[c + 2] - pos[a + 2];
    const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
    faceAw[3 * t] = nx; faceAw[3 * t + 1] = ny; faceAw[3 * t + 2] = nz;
    const l = Math.hypot(nx, ny, nz) || 1;
    faceUnit[3 * t] = nx / l; faceUnit[3 * t + 1] = ny / l; faceUnit[3 * t + 2] = nz / l;
  }
  const incident: number[][] = Array.from({ length: vcount }, () => []);
  for (let t = 0; t < tcount; t++)
    for (let k = 0; k < 3; k++) incident[idx[3 * t + k]].push(t);

  const find = (parent: number[], x: number): number => {
    while (parent[x] !== x) { parent[x] = parent[parent[x]]; x = parent[x]; }
    return x;
  };
  const outPos: number[] = [], outNrm: number[] = [];
  const cornerOut = new Uint32Array(idx.length);
  for (let v = 0; v < vcount; v++) {
    const faces = incident[v], k = faces.length;
    if (k === 0) continue;
    // Union incident faces within the crease angle (transitive → one group per
    // gradually-curving fillet). O(valence²); valences are small.
    const parent = Array.from({ length: k }, (_, i) => i);
    for (let i = 0; i < k; i++) for (let j = i + 1; j < k; j++) {
      const fi = faces[i] * 3, fj = faces[j] * 3;
      const d = faceUnit[fi] * faceUnit[fj] + faceUnit[fi + 1] * faceUnit[fj + 1] + faceUnit[fi + 2] * faceUnit[fj + 2];
      if (d >= CREASE_COS) parent[find(parent, i)] = find(parent, j);
    }
    const roots: number[] = [], ax: number[] = [], ay: number[] = [], az: number[] = [];
    for (let li = 0; li < k; li++) {
      const r = find(parent, li);
      let slot = roots.indexOf(r);
      if (slot < 0) { slot = roots.length; roots.push(r); ax.push(0); ay.push(0); az.push(0); }
      const f = faces[li] * 3;
      ax[slot] += faceAw[f]; ay[slot] += faceAw[f + 1]; az[slot] += faceAw[f + 2];
    }
    const base: number[] = [];
    for (let s = 0; s < roots.length; s++) {
      const l = Math.hypot(ax[s], ay[s], az[s]) || 1;
      base.push(outPos.length / 3);
      outPos.push(pos[3 * v], pos[3 * v + 1], pos[3 * v + 2]);
      outNrm.push(ax[s] / l, ay[s] / l, az[s] / l);
    }
    for (let li = 0; li < k; li++) {
      const slot = roots.indexOf(find(parent, li));
      const t = faces[li];
      const corner = idx[3 * t] === v ? 0 : idx[3 * t + 1] === v ? 1 : 2;
      cornerOut[3 * t + corner] = base[slot];
    }
  }
  return { positions: new Float32Array(outPos), normals: new Float32Array(outNrm), indices: cornerOut };
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
    const link = (vs: string, fs: string) => {
      const p = gl.createProgram()!;
      const mk = (t: number, s: string) => { const sh = gl.createShader(t)!; gl.shaderSource(sh, s); gl.compileShader(sh); return sh; };
      gl.attachShader(p, mk(gl.VERTEX_SHADER, vs)); gl.attachShader(p, mk(gl.FRAGMENT_SHADER, fs)); gl.linkProgram(p);
      return p;
    };
    const meshProg = link(MESH_VS, MESH_FS);
    const lineProg = link(LINE_VS, LINE_FS);

    const buf = (arr: BufferSource) => { const b = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, b); gl.bufferData(gl.ARRAY_BUFFER, arr, gl.STATIC_DRAW); return b; };
    const idxType = gl.getExtension("OES_element_index_uint") ? gl.UNSIGNED_INT : gl.UNSIGNED_SHORT;

    // One buffer set per build part, coloured by its assigned material.
    const meshes = parts.map((part) => {
      const cv = creaseVerts(part.positions as number[], part.indices as number[]);
      const idxBuf = gl.createBuffer()!;
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, idxBuf);
      gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, cv.indices, gl.STATIC_DRAW);
      return {
        posBuf: buf(cv.positions),
        nrmBuf: buf(cv.normals),
        idxBuf,
        count: cv.indices.length,
        color: EXTRUDER_RGB[part.extruder] ?? FALLBACK_RGB,
      };
    });

    gl.enable(gl.DEPTH_TEST);
    gl.clearColor(0.055, 0.07, 0.09, 1);

    // Combined model bbox across all parts, for framing.
    const lo = [1e9, 1e9, 1e9], hi = [-1e9, -1e9, -1e9];
    for (const part of parts) for (let k = 0; k < 3; k++) {
      lo[k] = Math.min(lo[k], part.bbox[k]); hi[k] = Math.max(hi[k], part.bbox[k + 3]);
    }
    const cx = (lo[0] + hi[0]) / 2, cy = (lo[1] + hi[1]) / 2, cz = (lo[2] + hi[2]) / 2;
    const { grid, border } = bedLines(cx, cy);
    const gridBuf = buf(grid), borderBuf = buf(border);

    // Orbit around the model, framed to show the whole build plate around it.
    const base = [cx, cy, cz];
    const o: Orbit = { az: 0.6, el: 0.5, dist: BED * 1.15, autoRotate: false, pan: [0, 0, 0] };
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
      const target = [base[0] + o.pan[0], base[1] + o.pan[1], base[2] + o.pan[2]];
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
      const nl = gl.getAttribLocation(meshProg, "aNormal");
      gl.uniformMatrix4fv(gl.getUniformLocation(meshProg, "uVP"), false, vp);
      gl.uniform3f(gl.getUniformLocation(meshProg, "uEye"), eye[0], eye[1], eye[2]);
      const uColor = gl.getUniformLocation(meshProg, "uColor");
      for (const m of meshes) {
        gl.bindBuffer(gl.ARRAY_BUFFER, m.posBuf); gl.enableVertexAttribArray(pl); gl.vertexAttribPointer(pl, 3, gl.FLOAT, false, 0, 0);
        gl.bindBuffer(gl.ARRAY_BUFFER, m.nrmBuf); gl.enableVertexAttribArray(nl); gl.vertexAttribPointer(nl, 3, gl.FLOAT, false, 0, 0);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, m.idxBuf);
        gl.uniform3f(uColor, m.color[0], m.color[1], m.color[2]);
        gl.drawElements(gl.TRIANGLES, m.count, idxType, 0);
      }

      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => { cancelAnimationFrame(raf); detach(); roz.disconnect(); };
  }, []);
  return <canvas ref={ref} style={{ position: "absolute", inset: 0, width: "100%", height: "100%", display: "block", background: "#0e1217" }} />;
}
