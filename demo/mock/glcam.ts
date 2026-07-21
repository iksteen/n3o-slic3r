// Shared minimal WebGL camera + orbit helpers for the demo's browser renderers.

export function perspective(fovy: number, aspect: number, near: number, far: number): number[] {
  const f = 1 / Math.tan(fovy / 2), nf = 1 / (near - far);
  return [f / aspect, 0, 0, 0, 0, f, 0, 0, 0, 0, (far + near) * nf, -1, 0, 0, 2 * far * near * nf, 0];
}
export function lookAt(eye: number[], ctr: number[], up: number[]): number[] {
  const z = norm(sub(eye, ctr)), x = norm(cross(up, z)), y = cross(z, x);
  return [x[0], y[0], z[0], 0, x[1], y[1], z[1], 0, x[2], y[2], z[2], 0,
    -dot(x, eye), -dot(y, eye), -dot(z, eye), 1];
}
export function mul(a: number[], b: number[]): number[] {
  const o = new Array(16);
  for (let c = 0; c < 4; c++) for (let r = 0; r < 4; r++) {
    let s = 0; for (let k = 0; k < 4; k++) s += a[k * 4 + r] * b[c * 4 + k];
    o[c * 4 + r] = s;
  }
  return o;
}
export const sub = (a: number[], b: number[]) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
export const cross = (a: number[], b: number[]) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
export const dot = (a: number[], b: number[]) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
export const norm = (a: number[]) => { const l = Math.hypot(a[0], a[1], a[2]) || 1; return [a[0] / l, a[1] / l, a[2] / l]; };

export function compile(gl: WebGLRenderingContext, type: number, src: string): WebGLShader {
  const sh = gl.createShader(type)!;
  gl.shaderSource(sh, src); gl.compileShader(sh);
  if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(sh) || "shader");
  return sh;
}
export function program(gl: WebGLRenderingContext, vs: string, fs: string): WebGLProgram {
  const p = gl.createProgram()!;
  gl.attachShader(p, compile(gl, gl.VERTEX_SHADER, vs));
  gl.attachShader(p, compile(gl, gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(p);
  return p;
}

/** Drag-orbit + wheel-zoom + idle auto-rotate. `az/el/dist` are read by the
 *  caller's render loop; `autoRotate` flips off on first interaction. */
export interface Orbit {
  az: number; el: number; dist: number; autoRotate: boolean;
}
export function attachOrbit(canvas: HTMLCanvasElement, o: Orbit, minDist: number, maxDist: number): () => void {
  let drag = false, px = 0, py = 0;
  const down = (e: PointerEvent) => { drag = true; o.autoRotate = false; px = e.clientX; py = e.clientY; canvas.setPointerCapture(e.pointerId); };
  const up = () => { drag = false; };
  const move = (e: PointerEvent) => {
    if (!drag) return;
    o.az += (e.clientX - px) * 0.008; o.el += (e.clientY - py) * 0.008;
    o.el = Math.max(-1.4, Math.min(1.4, o.el)); px = e.clientX; py = e.clientY;
  };
  const wheel = (e: WheelEvent) => { e.preventDefault(); o.dist *= Math.exp(e.deltaY * 0.001); o.dist = Math.max(minDist, Math.min(maxDist, o.dist)); };
  canvas.addEventListener("pointerdown", down);
  canvas.addEventListener("pointerup", up);
  canvas.addEventListener("pointermove", move);
  canvas.addEventListener("wheel", wheel, { passive: false });
  return () => {
    canvas.removeEventListener("pointerdown", down);
    canvas.removeEventListener("pointerup", up);
    canvas.removeEventListener("pointermove", move);
    canvas.removeEventListener("wheel", wheel);
  };
}
