// Dependency-free WebGL toolpath ("tube") preview. Consumes the global DATA
// object emitted by demo/extract_toolpaths.py: { features, bbox, layers:[{z,
// paths:[[featIdx, x0,y0, x1,y1, ...]]}] }. Renders each extrusion segment as a
// screen-space fat line with rounded shading (tube look), colored by feature,
// with a layer scrubber + a print-build animation + orbit camera.
"use strict";

// ---- Feature palette (slicer convention, tuned for a dark ground) ----
const FEATURE_COLORS = {
  "Outer wall": [1.0, 0.36, 0.24],
  "Inner wall": [0.24, 0.82, 0.44],
  "Overhang wall": [0.2, 0.8, 0.85],
  "Sparse infill": [0.85, 0.63, 0.27],
  "Internal solid infill": [0.9, 0.78, 0.28],
  "Gap infill": [0.95, 0.42, 0.72],
  "Top surface": [0.32, 0.66, 1.0],
  "Bottom surface": [0.2, 0.52, 0.86],
  "Internal Bridge": [0.7, 0.45, 1.0],
  Custom: [0.55, 0.6, 0.66],
};
const FALLBACK = [0.6, 0.66, 0.72];

// ---- tiny mat4 ----
function perspective(fovy, aspect, near, far) {
  const f = 1 / Math.tan(fovy / 2), nf = 1 / (near - far);
  return [f / aspect, 0, 0, 0, 0, f, 0, 0, 0, 0, (far + near) * nf, -1, 0, 0, 2 * far * near * nf, 0];
}
function lookAt(eye, ctr, up) {
  const z = norm(sub(eye, ctr)), x = norm(cross(up, z)), y = cross(z, x);
  return [x[0], y[0], z[0], 0, x[1], y[1], z[1], 0, x[2], y[2], z[2], 0,
    -dot(x, eye), -dot(y, eye), -dot(z, eye), 1];
}
function mul(a, b) {
  const o = new Array(16);
  for (let c = 0; c < 4; c++) for (let r = 0; r < 4; r++) {
    let s = 0; for (let k = 0; k < 4; k++) s += a[k * 4 + r] * b[c * 4 + k];
    o[c * 4 + r] = s;
  }
  return o;
}
const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const norm = (a) => { const l = Math.hypot(a[0], a[1], a[2]) || 1; return [a[0] / l, a[1] / l, a[2] / l]; };

// ---- build instance buffers from DATA ----
function buildBuffers(data) {
  const starts = [], ends = [], colors = [], layerIdx = [];
  data.layers.forEach((ly, li) => {
    for (const path of ly.paths) {
      const col = FEATURE_COLORS[data.features[path[0]]] || FALLBACK;
      for (let i = 1; i + 3 < path.length; i += 2) {
        starts.push(path[i], path[i + 1], ly.z);
        ends.push(path[i + 2], path[i + 3], ly.z);
        colors.push(col[0], col[1], col[2]);
        layerIdx.push(li);
      }
    }
  });
  return {
    starts: new Float32Array(starts), ends: new Float32Array(ends),
    colors: new Float32Array(colors), layerIdx: new Float32Array(layerIdx),
    count: layerIdx.length,
  };
}

const VS = `
attribute vec2 aCorner;      // (along 0..1, side -1..1)
attribute vec3 aStart, aEnd, aColor;
attribute float aLayer;
uniform mat4 uVP;
uniform vec2 uViewport;
uniform float uWidth, uMaxLayer;
varying vec3 vColor;
varying float vSide;
void main() {
  if (aLayer > uMaxLayer) { gl_Position = vec4(2.0, 2.0, 2.0, 1.0); return; }
  vec4 cs = uVP * vec4(aStart, 1.0);
  vec4 ce = uVP * vec4(aEnd, 1.0);
  vec4 clip = mix(cs, ce, aCorner.x);
  vec2 s = cs.xy / cs.w, e = ce.xy / ce.w;
  vec2 dir = normalize((e - s) * uViewport + vec2(1e-6, 0.0));
  vec2 nrm = vec2(-dir.y, dir.x) / uViewport;
  clip.xy += nrm * aCorner.y * uWidth * clip.w;
  gl_Position = clip;
  vColor = aColor;
  vSide = aCorner.y;
}`;

const FS = `
precision mediump float;
varying vec3 vColor;
varying float vSide;
void main() {
  float a = 1.0 - abs(vSide);           // 1 at tube center, 0 at edge
  float shade = 0.5 + 0.5 * sqrt(max(a, 0.0));
  gl_FragColor = vec4(vColor * shade, 1.0);
}`;

function compile(gl, type, src) {
  const sh = gl.createShader(type);
  gl.shaderSource(sh, src); gl.compileShader(sh);
  if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(sh));
  return sh;
}
function instAttr(gl, prog, name, buf, size) {
  const loc = gl.getAttribLocation(prog, name);
  gl.bindBuffer(gl.ARRAY_BUFFER, buf);
  gl.enableVertexAttribArray(loc);
  gl.vertexAttribPointer(loc, size, gl.FLOAT, false, 0, 0);
  ext.vertexAttribDivisorANGLE(loc, 1);
}

let gl, ext, prog, buffers, uni, corner;

function init(canvas, data) {
  gl = canvas.getContext("webgl", { antialias: true, alpha: false });
  ext = gl.getExtension("ANGLE_instanced_arrays");
  prog = gl.createProgram();
  gl.attachShader(prog, compile(gl, gl.VERTEX_SHADER, VS));
  gl.attachShader(prog, compile(gl, gl.FRAGMENT_SHADER, FS));
  gl.linkProgram(prog);
  gl.useProgram(prog);

  const b = buildBuffers(data);
  const mk = (arr) => { const bb = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, bb); gl.bufferData(gl.ARRAY_BUFFER, arr, gl.STATIC_DRAW); return bb; };
  buffers = { start: mk(b.starts), end: mk(b.ends), color: mk(b.colors), layer: mk(b.layerIdx), count: b.count };

  corner = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, corner);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, -1, 1, -1, 1, 1, 0, -1, 1, 1, 0, 1]), gl.STATIC_DRAW);

  uni = {
    VP: gl.getUniformLocation(prog, "uVP"),
    viewport: gl.getUniformLocation(prog, "uViewport"),
    width: gl.getUniformLocation(prog, "uWidth"),
    maxLayer: gl.getUniformLocation(prog, "uMaxLayer"),
  };
  gl.enable(gl.DEPTH_TEST);
  gl.clearColor(0.055, 0.07, 0.09, 1);
}

function draw(vp, viewport, maxLayer) {
  gl.viewport(0, 0, viewport[0], viewport[1]);
  gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
  gl.useProgram(prog);
  const cl = gl.getAttribLocation(prog, "aCorner");
  gl.bindBuffer(gl.ARRAY_BUFFER, corner);
  gl.enableVertexAttribArray(cl);
  gl.vertexAttribPointer(cl, 2, gl.FLOAT, false, 0, 0);
  ext.vertexAttribDivisorANGLE(cl, 0);
  instAttr(gl, prog, "aStart", buffers.start, 3);
  instAttr(gl, prog, "aEnd", buffers.end, 3);
  instAttr(gl, prog, "aColor", buffers.color, 3);
  instAttr(gl, prog, "aLayer", buffers.layer, 1);
  gl.uniformMatrix4fv(uni.VP, false, vp);
  gl.uniform2f(uni.viewport, viewport[0], viewport[1]);
  gl.uniform1f(uni.width, 3.0);
  gl.uniform1f(uni.maxLayer, maxLayer);
  ext.drawArraysInstancedANGLE(gl.TRIANGLES, 0, 6, buffers.count);
}

// ---- app ----
(function () {
  const data = window.DATA;
  const canvas = document.getElementById("view");
  init(canvas, data);

  const bb = data.bbox;
  const center = [(bb[0] + bb[3]) / 2, (bb[1] + bb[4]) / 2, (bb[2] + bb[5]) / 2];
  const radius = Math.hypot(bb[3] - bb[0], bb[4] - bb[1], bb[5] - bb[2]) / 2;

  let az = 0.7, el = 0.5, dist = radius * 2.6;
  let autoRotate = true;
  const slider = document.getElementById("layer");
  const layerLabel = document.getElementById("layerlabel");
  const N = data.layers.length;
  slider.max = String(N);
  let maxLayer = 0, building = true, buildT = 0;

  function resize() {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.floor(canvas.clientWidth * dpr);
    canvas.height = Math.floor(canvas.clientHeight * dpr);
  }
  window.addEventListener("resize", resize);
  resize();

  // pointer orbit
  let drag = false, px = 0, py = 0;
  canvas.addEventListener("pointerdown", (e) => { drag = true; autoRotate = false; px = e.clientX; py = e.clientY; canvas.setPointerCapture(e.pointerId); });
  canvas.addEventListener("pointerup", () => { drag = false; });
  canvas.addEventListener("pointermove", (e) => {
    if (!drag) return;
    az += (e.clientX - px) * 0.008; el += (e.clientY - py) * 0.008;
    el = Math.max(-1.5, Math.min(1.5, el)); px = e.clientX; py = e.clientY;
  });
  canvas.addEventListener("wheel", (e) => { e.preventDefault(); dist *= Math.exp(e.deltaY * 0.001); dist = Math.max(radius * 1.2, Math.min(radius * 6, dist)); }, { passive: false });

  slider.addEventListener("input", () => { building = false; maxLayer = Number(slider.value); autoRotate = false; });
  document.getElementById("replay").addEventListener("click", () => { building = true; buildT = 0; autoRotate = true; });

  let last = performance.now();
  function frame(now) {
    const dt = Math.min((now - last) / 1000, 0.05); last = now;
    if (building) { buildT += dt; maxLayer = Math.min(N, buildT * N / 3.0); if (maxLayer >= N) building = false; slider.value = String(Math.round(maxLayer)); }
    if (autoRotate) az += dt * 0.25;
    layerLabel.textContent = `layer ${Math.round(maxLayer)} / ${N}`;

    const eye = [
      center[0] + dist * Math.cos(el) * Math.sin(az),
      center[1] + dist * Math.cos(el) * Math.cos(az),
      center[2] + dist * Math.sin(el),
    ];
    const aspect = canvas.width / canvas.height;
    const vp = mul(perspective(0.9, aspect, radius * 0.05, radius * 20), lookAt(eye, center, [0, 0, 1]));
    draw(vp, [canvas.width, canvas.height], maxLayer);
    requestAnimationFrame(frame);
  }

  // legend
  const legend = document.getElementById("legend");
  data.features.forEach((f) => {
    const c = FEATURE_COLORS[f] || FALLBACK;
    const el2 = document.createElement("div");
    el2.className = "legend-item";
    el2.innerHTML = `<span class="sw" style="background:rgb(${(c[0] * 255) | 0},${(c[1] * 255) | 0},${(c[2] * 255) | 0})"></span>${f}`;
    legend.appendChild(el2);
  });

  requestAnimationFrame(frame);
})();
