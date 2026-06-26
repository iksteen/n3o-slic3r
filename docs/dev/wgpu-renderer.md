# Decision record — replace the Three.js viewport with a Rust-side wgpu renderer

**Status: IMPLEMENTED / SHIPPED.** Adopted **option B — one wgpu renderer**,
**Strategy A** (offscreen wgpu → opaque `<canvas>` blit): the prepare-tab edit
viewport is now the wgpu renderer; the Three.js prepare viewport is deleted.
Original honest effort estimate: **~5–7 weeks**.

**Implemented.** Feature parity reached: meshes, spool colors, per-face MMU
paint, selection, picking, gizmos (move/rotate/scale with snap), placing tools
(lay-flat, align X/Y, match-face), clone, auto-orient, arrange, axis markers,
priming tower (box + sliced mesh, draggable), print thumbnail, MSAA, and camera
framing all match the old viewport. The Three.js prepare cluster (`ViewportCanvas`,
`sceneMirror`, `gizmo.ts`, `cameraControls`, `eventBridge`, `paintColors`,
`thumbnail.ts`, `towerOverlay`) and the backend `scene_mesh_buffers` /
`scene_mesh_paint` commands were removed. Mesh geometry never crosses the IPC
bridge — it is uploaded straight to the GPU Rust-side. The G-code preview
(`src-tauri/src/toolpath_render.rs` + `src/preview/GcodePreview.tsx`) is now
also a wgpu renderer (instanced tubes, same Strategy-A blit); no Three.js
remains anywhere in the app.

**Linux present path — corrected after phase-1 implementation: Strategy A
(offscreen wgpu → opaque `<canvas>` blit), NOT GtkGLArea.** The GtkGLArea path
(wgpu composited behind a transparent webview) was throughput- and feel-validated
on real Intel hardware, but it requires a **transparent WebKitGTK webview**, and
that does not work for our UI: WebKitGTK does not clear removed DOM in transparent
mode, so dynamic overlays (tooltips, toasts, the progress window, the console)
that land over the viewport **smear** — their pixels persist when dismissed. See
"Phase-1 finding: transparency is dead on Linux" below. Strategy A needs no
transparency (opaque canvas, DOM overlays composite normally) at the cost of a
per-frame copy — acceptable at edit-viewport sizes.

**Scope, as it landed:** the edit viewport shipped first; the G-code preview was
then migrated to its own wgpu renderer (`toolpath_render.rs`) as a separate
follow-up — exactly the redesign anticipated here. Both screens now run wgpu, the
interim two-renderer (wgpu + Three.js) split is over, and the Three.js/WebGL
stack is fully removed. The ~8–12 wk "full parity" figure below was the original
two-renderer estimate, kept for historical reference.

This record consolidates six spikes. The raw measurement harnesses, per-spike
`RESULTS.md` files, and the `gtk-glarea-stl` Linux-present bridge (the code the
port copies from) are archived on branch `spike/wgpu-spike0`, under `spikes/`.
It supersedes the 2–3 week figure in PRD §10's risk row, which only ever covered
the *state* axis.

---

## 1. The problem

Orbiting a loaded plate is laggy on Linux. Root cause: **WebKitGTK has no GPU
path for the webview's WebGL** — the dmabuf renderer crashes, so we ship
`WEBKIT_DISABLE_DMABUF_RENDERER=1`, which drops it to **software triangle
rasterization on the CPU**. WebGPU-in-webview would be the in-place fix, but
**WebKitGTK has no WebGPU** — so there is *no* in-webview GPU upgrade path on the
primary platform. The only way past the software-GL ceiling is a native GPU
renderer (wgpu).

On-demand rendering (already shipped) masked the *idle* CPU/fan symptom but not
the orbit lag — the moment you drag, every frame is software-rastered again.
macOS/Windows don't have this problem (WKWebView/WebView2 GPU-accelerate the
existing renderer), so the pain is Linux-specific — **but the fix is not**
(see §3, the RTX result).

---

## 2. What the spikes proved

| # | Spike | Question | Result |
| --- | --- | --- | --- |
| 0 | `wgpu-readback` | Is wgpu render + GPU→CPU readback cheap enough for Strategy A? | **PASS** — 4K render+readback ~1.5 ms on RTX (≈10× under the 60fps budget); resolution-bound, not scene-bound. |
| B | `tauri-native-window` | Can a raw wgpu swapchain share a Tauri window's surface with the webview? | **DEAD on Linux** — Wayland: `Gdk Error 71` (two clients, one `wl_surface`); X11: wgpu present occludes the webview. Env-var workarounds don't help (crash is upstream of WebKit). |
| — | `webview-blit` | How fast is the Strategy-A frame transport *to* the webview? | **Transport wall** — custom URI scheme: WebKitGTK ~330 MB/s, WebView2-in-VM ~95 MB/s, WKWebView ~5 GB/s. Linux/Windows transport blows the 4K budget; only macOS is fine. |
| — | `gtk-glarea` | Can GTK-composited GPU content coexist with the webview in one window? | **YES** — a `GtkGLArea` packed into Tauri's `default_vbox()` renders beside the webview on Wayland, no crash (it's a GTK widget GTK composites, not a swapchain fighting for the surface). |
| — | `wgpu-mesh-fps` | Does the iGPU rasterize a representative scene fast enough? | **PASS, decisively** — weakest modern Intel iGPU (J5005 UHD 605): 100K tris @385fps, 1M @195fps, 4M @60fps. Iris Xe is several× that. |
| — | `gtk-glarea-stl` | Built the real bridge + the interactive **feel** test. | **PASS** — wgpu adopts the GLArea's GL context (zero-copy), renders an STL on a plate, orbit/zoom. On the **Intel Iris Xe laptop**: "incredibly smooth, the difference is staggering" vs the WebKitGTK viewport. |

Two ideas were *ruled out* by this, not just deprioritized:
- **Strategy B** (native-window zero-copy): dead on Linux. There is no
  single-surface native composite with a GTK webview.
- **"Strategy A everywhere"** (offscreen → webview canvas): killed by the
  transport wall on the two platforms that would need it (Linux, Windows).

---

## 3. The architecture

**One renderer, one cross-platform present path.** A renderer is the whole scene
system (meshes, materials, spool-color chain, MMU paint, selection, gizmo,
picking, bed overlay). It's wgpu *everywhere*. The shipped present path —
**Strategy A** (offscreen wgpu → readback → opaque `<canvas>` blit) — is pure
offscreen render + readback with no per-platform code (`viewport_render.rs` has
no `cfg(target_os)`, no `CAMetalLayer`/DXGI), so it already works on all three
platforms today. The zero-copy native paths below are *optional optimizations*,
not prerequisites for the viewport to function:

| platform | present | status |
| --- | --- | --- |
| **all** | **Strategy A** — offscreen wgpu → readback → opaque `<canvas>` blit (the GtkGLArea path is dead — needs webview transparency; see the transparency finding below) | shipped, cross-platform |
| **macOS** | optional: wgpu → `CAMetalLayer` (zero-copy) instead of the readback (~5 GB/s measured) | optional optimization, not built |
| **Windows** | optional: DXGI swapchain / DirectComposition instead of the readback | optional optimization, not built |

**Why the GtkGLArea path was so attractive (and why it still lost):**
`gtk-glarea-stl` is visibly smoother than the WebKitGTK viewport **even on an
RTX 5070 Ti** — a GPU nowhere near its limit. So the Linux bottleneck is WebKit's
*CPU* software-raster, not GPU class; every Linux user benefits, not just the
weak-iGPU case. The GtkGLArea bridge (wgpu adopts the widget's GL context via
`wgpu_hal::gles::Adapter::new_external`, renders offscreen, `glBlitFramebuffer`
into GTK's FBO — zero readback) was built and felt "staggering" on the Intel
laptop. But presenting it *behind* the DOM requires a **transparent webview**,
which WebKitGTK can't do without smearing dynamic overlays — so it's out for the
real UI (see the transparency finding below). Strategy A keeps the GPU-rasterization win (the heavy part) and
pays a per-frame copy for present; it loses the zero-copy compositing, but it's
the only path that coexists with the dynamic DOM overlays.

**The AD-8 dividend.** Scene *state* (geometry, transforms, selection, MMU paint,
spool colors) already lived in Rust (`core/scene/`). A wgpu renderer consumes that
in-process — the state half of the port was genuinely free, which is what the old
2–3 wk estimate was really measuring. In the shipped renderer the mesh geometry no
longer crosses the IPC bridge at all (the binary `scene_mesh_buffers` IPC was
removed); it is uploaded straight to the GPU Rust-side, with `scene:*` events
driving updates.

---

## Phase-1 finding: transparency is dead on Linux (WebKitGTK)

Phase 1 set out to retire the one unretired risk — the **hole-punch layout**: a
transparent webview on top, GPU content showing through the viewport region. The
compositing half *works* (a `GtkGLArea` behind a transparent webview shows
through — orange visible in the viewport, panels opaque, input intact). The
**repaint** half does not, and it kills the approach:

**WebKitGTK does not clear its transparent surface when DOM is removed.** A
dynamic transient drawn over the viewport (a tooltip at the cursor, a toast, the
slicing progress window, the console drawer) **leaves its pixels behind** when
dismissed — it smears. These overlays land *anywhere* over the viewport and are
intrinsic to the UI, so this is fatal, not cosmetic.

Ruled out exhaustively (each tested on the running app):
- **Every way of enabling transparency is the same call on Linux.** wry 0.55
  `webkitgtk/mod.rs:289` is *ungated*: `if attributes.transparent {
  webview.set_background_color(0,0,0,0) }`. The `transparent` Cargo feature only
  gates the macOS/Windows paths. So config `transparent`, runtime
  `set_background_color`, and builder `.transparent(true)` are byte-for-byte
  identical here — and all smear.
- **X11 and Wayland both smear** (`GDK_BACKEND=x11` included).
- **`WEBKIT_DISABLE_COMPOSITING_MODE=1`** — no help.
- **Smears over the bare desktop** (transparent webview, no GLArea behind) — so
  it's WebKit's own surface, not the GTK overlay/GLArea compositing.
- **wry's `examples/wgpu.rs` doesn't disprove it**: it `panic!`s on Wayland, and
  on X11 it never adds/removes DOM — it only demonstrates *static* transparent
  compositing (which we also have). It never exercises clear-on-removal.

The alternative that needs no transparency — **GtkGLArea *on top* of an opaque
webview** — is also out: the dynamic overlays above must render *over* the 3D, and
an opaque GL widget on top hides them (and hiding the GL while any transient is up
would flash the viewport away constantly).

**Conclusion:** on Linux/WebKitGTK there is no way to composite native GPU content
*under* dynamic DOM. The 3D must live *inside* the webview as an opaque surface →
**Strategy A** (offscreen wgpu → `<canvas>` blit). It forfeits zero-copy present
(per-frame readback + transport, ~330 MB/s on WebKitGTK — fine at edit-viewport
sizes, degrades at 4K) but keeps the GPU-rasterization win and lets every dynamic
overlay composite normally over an ordinary canvas.

---

## 4. Scope & effort (the honest version)

The renderer *behavior* is the work — ~6.4k LOC across two renderers today, plus
everything Three.js gives for free (orbit damping, `TransformControls`, CPU
raycasting, text labels):

| frontend surface | LOC | wgpu equivalent | difficulty |
| --- | --- | --- | --- |
| `cameraControls.ts` | 130 | glam camera math | trivial |
| `sceneMirror.ts` (event→mesh, spool-color chain, selection tint, MMU per-face, bed/axes/exclusion overlays) | ~985 | scene mirror + materials + overlay geom | moderate–hard |
| `ViewportCanvas.tsx` (5 pick modes, bed-plane raycast, input, loop) | ~1,396 | render graph + CPU ray/triangle picker + input glue | hard |
| `gizmo.ts` (T/R/S, multi-select pivot, snap) | ~296 | **hand-rolled gizmo** (no off-the-shelf) | hard — biggest single long pole |
| `thumbnail.ts` (iso PNG for `.gcode.3mf`/U1) | ~138 | headless wgpu offscreen → PNG | moderate |
| `towerOverlay.ts` / `paintColors.ts` / axis text | ~250 | overlays + **glyph pipeline** (no DOM text) | moderate |
| **`src/preview/*`** — the SECOND renderer (G-code preview, custom segment shader, hover raycast, layer windowing) | **~3,080** | second wgpu renderer (`toolpath_render.rs`) | **done** |

**Edit-viewport-only ≈ 5–7 wk. Full parity (both renderers, 3 platforms) ≈ 8–12 wk.**
Picking moved from CPU-side JS (`Raycaster` + `Plane`) to a Rust CPU
ray/triangle scan (Möller–Trumbore over every triangle — `nearest_hit` in
`viewport_render.rs`; a BVH is the obvious upgrade if a high-poly scene ever lags
picking), with face-normal parity for the lay-flat/auto-orient/align modes.

---

## 5. Phased plan (Strategy A, edit viewport first)

Steps 1–6 are the edit-viewport work that shipped; step 8 (the preview renderer)
shipped since as a follow-up; step 7 is an optional optimization.

1. **Foundation** *(done)* — Strategy-A present loop: wgpu renders the scene to an
   offscreen texture sized to the viewport, reads it back, and serves the bytes
   to JS; the frontend draws them into an opaque `<canvas>` where the Three.js
   renderer used to be. Camera/input stay in JS → forwarded to Rust; Rust
   re-renders on change (on-demand). glam camera consuming `scene:*`.
   *(The hole-punch / GtkGLArea path was retired here — see the transparency
   finding above.)*
2. **Scene parity** *(done)* — meshes + lighting + bed/axes/exclusion overlays +
   spool-color chain + MMU per-face paint + selection tint, with geometry
   uploaded GPU-side rather than over the (now-removed) `scene_mesh_buffers` IPC.
3. **Picking + selection** *(done)* — Rust CPU ray/triangle scan (`nearest_hit`,
   Möller–Trumbore); closest-hit object + face-pick world normal parity vs the
   old viewport across all 5 modes.
4. **Gizmo** *(done)* — hand-rolled T/R/S, multi-select pivot, snapping (was the
   longest pole).
5. **Overlays + text** *(done)* — tower overlay, dimension/axis labels (glyph
   pipeline).
6. **Thumbnail** *(done)* — headless wgpu offscreen → PNG for `.gcode.3mf`/U1.
7. **Cross-platform present optimizations** *(optional, not built)* — CAMetalLayer
   (macOS), DXGI/DComp (Windows) to replace the readback with a zero-copy present.
   Strategy A already works cross-platform, so these are only a perf optimization.
8. **G-code preview renderer** *(done)* — migrated to its own wgpu renderer
   (`toolpath_render.rs`, instanced tubes); the Three.js/WebGL stack is fully
   removed. (Preview is a separate screen, so it was a clean standalone follow-up.)

---

## 6. Open items / risks

- **Hole-punch layout** — *resolved: dead* (transparency finding above).
  Strategy A replaced it. The Linux risk it raised — per-frame transport cost at
  large/HiDPI viewport sizes — was mitigated as planned (render-at-viewport-size +
  DPR 1 + on-demand) and held up at edit-viewport sizes in the shipped renderer.
- **Windows present path** — Strategy A (readback) works cross-platform and is the
  shipped path; a zero-copy DXGI/DComp present remains an optional optimization,
  only ever measured in a GPU-less VM (not on the critical path).
- **Preview renderer** — *done*: migrated to its own wgpu renderer
  (`toolpath_render.rs`) after the prepare tab landed; the Three.js/WebGL stack is
  fully removed.
- **Gizmo** — no off-the-shelf Rust equivalent to `TransformControls`; was the
  largest single task, now shipped.
- **flatpak** — moot: the GtkGLArea path is dead, so there's no GTK GPU widget to
  sandbox. Strategy A renders offscreen with wgpu inside the webview; the flatpak
  channel ships.

---

## 7. Recommendation

The recommendation was option B for the **edit viewport** (~5–7 wk), with the
preview renderer as a separate follow-up. That is what shipped, and then some:
the edit-viewport port (§5 steps 1–6) is complete at feature parity, the Three.js
prepare cluster is deleted, and the preview was subsequently migrated to its own
wgpu renderer too (§5 step 8) — so no Three.js remains. Every kill-criterion spike
passed and the highest-risk piece (the Linux present path) landed on Strategy A
after the transparency finding retired the GtkGLArea/transparent-webview route.
The only remaining work is the optional zero-copy present optimizations (§5 step
7), which Strategy A makes non-blocking.
