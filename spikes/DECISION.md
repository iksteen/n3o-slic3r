# Decision record — replace the Three.js viewport with a Rust-side wgpu renderer

**Status: GO.** Adopt **option B — one wgpu renderer**, delete the **edit
viewport's** Three.js (`src/viewport/`) and replace it with wgpu. The Linux
present path is **wgpu → GtkGLArea, zero-copy**, validated end-to-end on real
Intel hardware. Honest effort: **~5–7 weeks**.

**Scope decided:** the G-code preview (`src/preview/`) **stays on Three.js for
now** — it gets its own full redesign + wgpu rewrite *after* the prepare tab is
finished, not as part of this port. So the edit viewport and the preview run
different renderers for an interim period (acceptable: they're separate screens,
not kept in sync). The ~8–12 wk "full parity" figure below is the eventual
two-renderer total, for reference — it is **not** this project's scope.

This record consolidates six spikes (branch `spike/wgpu-spike0`, worktree
`../n3o-slic3r-wgpu`, dir `spikes/`). It supersedes the 2–3 week figure in PRD
§10's risk row, which only ever covered the *state* axis.

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

**One renderer, thin per-platform present shim.** A renderer is the whole scene
system (meshes, materials, spool-color chain, MMU paint, selection, gizmo,
picking, bed overlay). You cannot keep Three.js on mac/win and wgpu on Linux —
that's two full renderers kept in sync forever. So it's wgpu *everywhere*; only
how the finished frame reaches the screen differs:

| platform | present shim | status |
| --- | --- | --- |
| **Linux** | **wgpu → GtkGLArea** (adopt the widget's GL context, render offscreen, `glBlitFramebuffer` into GTK's FBO) | **built + validated** (`gtk-glarea-stl`) |
| **macOS** | wgpu → `CAMetalLayer` (native, zero-copy), or Strategy A (~5 GB/s measured — also fine) | not built; both paths known-good |
| **Windows** | DXGI swapchain on a child HWND / DirectComposition | not built; needs real-hw check |

The Linux bridge — the previously-unbuilt, highest-risk piece — is done:
1. In `connect_render`, `wgpu_hal::gles::Adapter::new_external` builds a wgpu
   adapter on the GLArea's *own* current GL context (loader =
   `epoxy::get_proc_addr`; libepoxy exports `epoxy_*` dispatch pointers, not
   plain `glFoo`).
2. wgpu renders the scene into an offscreen texture (its own depth, lighting).
3. `glBlitFramebuffer` composites that texture into GTK's framebuffer — captured
   at frame-top, because wgpu's submit leaves *its* FBO bound. **No CPU
   readback**; the texture never leaves the GPU.

**Why the win generalizes (the key reframe):** `gtk-glarea-stl` is visibly
smoother than the WebKitGTK viewport **even on an RTX 5070 Ti** — a GPU nowhere
near its limit. So the Linux bottleneck is WebKit's *CPU* software-raster, not
GPU class. Every Linux user benefits — discrete GPUs included — not just the
weak-iGPU case. The Intel laptop then confirmed it on the actual target.

**The AD-8 dividend.** Scene *state* (geometry, transforms, selection, MMU paint,
spool colors) already lives in Rust (`core/scene/`), exposed via the binary
`scene_mesh_buffers` IPC + `scene:*` events. A wgpu renderer consumes that
unchanged — the state half of the port is genuinely free, which is what the old
2–3 wk estimate was really measuring.

---

## 4. Scope & effort (the honest version)

The renderer *behavior* is the work — ~6.4k LOC across two renderers today, plus
everything Three.js gives for free (orbit damping, `TransformControls`, CPU
raycasting, text labels):

| frontend surface | LOC | wgpu equivalent | difficulty |
| --- | --- | --- | --- |
| `cameraControls.ts` | 130 | glam camera math | trivial |
| `sceneMirror.ts` (event→mesh, spool-color chain, selection tint, MMU per-face, bed/axes/exclusion overlays) | ~985 | scene mirror + materials + overlay geom | moderate–hard |
| `ViewportCanvas.tsx` (5 pick modes, bed-plane raycast, input, loop) | ~1,396 | render graph + **BVH picker** + input glue | hard |
| `gizmo.ts` (T/R/S, multi-select pivot, snap) | ~296 | **hand-rolled gizmo** (no off-the-shelf) | hard — biggest single long pole |
| `thumbnail.ts` (iso PNG for `.gcode.3mf`/U1) | ~138 | headless wgpu offscreen → PNG | moderate |
| `towerOverlay.ts` / `paintColors.ts` / axis text | ~250 | overlays + **glyph pipeline** (no DOM text) | moderate |
| **`src/preview/*`** — the SECOND renderer (G-code preview, custom segment shader, hover raycast, layer windowing) | **~3,080** | second wgpu renderer | hard — the omitted half |

**Edit-viewport-only ≈ 5–7 wk. Full parity (both renderers, 3 platforms) ≈ 8–12 wk.**
Picking is CPU-side JS today (`Raycaster` + `Plane`); wgpu needs a Rust
ray/triangle BVH — net-new, and the lay-flat/auto-orient/align modes need
face-normal parity with three.js.

---

## 5. Phased plan (Strategy A/GtkGLArea, edit viewport first)

1. **Foundation** — GtkGLArea present shim into the real app's GTK window
   (`default_vbox()` + the `gtk-glarea-stl` bridge), `invalidate()`-driven
   `queue_render()`, glam camera consuming `scene:*`. *Adds: the hole-punch
   layout — transparent webview over the GLArea so React panels frame a central
   GL region. Unverified: transparent WebKitGTK over a GL widget on Wayland — do
   this early, it's the one remaining Linux integration unknown.*
2. **Scene parity** — meshes + lighting + bed/axes/exclusion overlays +
   spool-color chain + MMU per-face paint + selection tint, from
   `scene_mesh_buffers`.
3. **Picking + selection** — Rust BVH; closest-hit object + face-pick world
   normal parity vs three.js across all 5 modes.
4. **Gizmo** — hand-rolled T/R/S, multi-select pivot, snapping (longest pole).
5. **Overlays + text** — tower overlay, dimension/axis labels (glyph pipeline).
6. **Thumbnail** — headless wgpu offscreen → PNG for `.gcode.3mf`/U1.
7. **Cross-platform present shims** — CAMetalLayer (macOS), DXGI/DComp
   (Windows); verify Windows on real hardware.
8. **G-code preview renderer — OUT of this project.** Stays on Three.js until
   the prepare tab is done, then gets its own full redesign + wgpu rewrite. The
   WebGL stack therefore lingers for the preview screen in the interim — known
   and accepted. (Preview is a separate screen, so the interim two-renderer
   split costs nothing in sync.)

---

## 6. Open items / risks

- **Hole-punch layout** (transparent webview over GLArea on Wayland/flatpak) —
  the one Linux integration unknown left; retire it in phase 1. The
  coexistence half is already proven (`gtk-glarea`); transparency + input
  z-order is not.
- **Windows present path** — only ever measured in a GPU-less VM. Needs a
  real-hardware DXGI/DComp check (not on the Linux critical path).
- **Preview renderer** — *decided OUT* (stays Three.js → own redesign + wgpu
  rewrite after the prepare tab). Not a risk for this project; noted so the
  interim two-renderer split is a conscious choice, not drift.
- **Gizmo** — no off-the-shelf Rust equivalent to `TransformControls`; budget it
  as the largest single task.
- **flatpak** — the GtkGLArea path must work inside the sandbox (same stack that
  needs the dmabuf workaround today); validate in the flatpak build, not just a
  bare run.

---

## 7. Recommendation

Commit to option B for the **edit viewport** (~5–7 wk); the preview renderer is
explicitly out (Three.js until the prepare tab lands, then its own wgpu rewrite).
Every kill-criterion spike passed and the highest-risk piece (the Linux zero-copy
present bridge) is built and feels "staggering" on the actual Intel target.
Sequence the work as §5; the one remaining Linux unknown is the
transparent-webview-over-GLArea check — gate phase 1 on it.
