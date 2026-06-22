# wgpu renderer — multi-platform picture (measured)

The native-window path (GtkGLArea) is Linux-only and doesn't port — GTK doesn't
exist on macOS/Windows. The question was whether that's a problem. **It mostly
isn't**, because the thing that *motivated* the native path — the ~330 MB/s
frame-transport wall — is **WebKitGTK-specific**.

## Strategy A (offscreen wgpu → present to webview canvas), measured

Same benchmark crate (`webview-blit`), same code, two platforms:

| stage | Linux WebKitGTK (RTX 5070 Ti) | macOS WKWebView (Apple M4) | Windows WebView2 (VM, no GPU) |
| --- | --- | --- | --- |
| wgpu render + readback @4K | ~1.56 ms (Vulkan) | ~1.34 ms (Metal) | not measured (no GPU) |
| `putImageData` @4K | 2–3 ms | 3 ms | 2.5 ms |
| present cadence (all res) | 60 fps (vsync) | 60 fps (vsync) | 61–63 fps (vsync) |
| **transport, custom protocol @4K** | 93 ms (**~340 MB/s**) | 6.5 ms (**~4.9 GB/s**) | 326 ms (**~97 MB/s**) |
| **transport, custom protocol @1080p** | 25 ms (~313 MB/s) | 1.6 ms (~4.9 GB/s) | 78 ms (~101 MB/s) |

Transport spread is ~50×: macOS ~5 GB/s, Linux ~330 MB/s, Windows-in-VM ~95 MB/s
(release build — debug ruled out; both invoke and custom protocol the same). On
macOS, end-to-end @4K (readback 1.3 + transport 6.5 + putImageData 3 ≈ 11 ms) <
16.7 ms → **60 fps via plain Strategy A, no native compositing**. On Linux and
Windows the transport alone blows the 4K budget.

### Big caveat on Windows — and the reframe it forces

The Windows number is from a **GPU-less VM**, so treat it as provisional. But the
more important point it surfaces: **the actual 3D-perf problem is
Linux/WebKitGTK-specific in the first place.** WebKitGTK on Linux has *no* GPU
acceleration for the webview's WebGL (we ship `WEBKIT_DISABLE_DMABUF_RENDERER=1`
because the GPU path crashes) → the existing Three.js renderer runs on a
**software** path → slow (the fans). **WKWebView (macOS) and WebView2 (Windows)
GPU-accelerate in-webview WebGL on real hardware** → the *existing* Three.js
renderer is already fine there. Nobody reported perf problems on macOS/Windows;
only Linux.

So "ship frames *to* the webview" (Strategy A) being slow on Windows/Linux is
mostly moot: on macOS/Windows you'd never do it — you'd just keep rendering in
the (GPU-accelerated) webview. You only need an alternative on **Linux**, where
the webview itself is the slow software path.

## Where the perf problem lives

| platform | webview | in-webview WebGL (today's Three.js renderer) |
| --- | --- | --- |
| **macOS** | WKWebView | GPU-accelerated (Metal) → fine |
| **Windows** | WebView2 | GPU-accelerated (D3D) on real hw → fine |
| **Linux** | WebKitGTK | **software** (GPU path crashes → `WEBKIT_DISABLE_DMABUF_RENDERER=1`) → slow |

The 3D-perf symptom (fans/CPU) is **Linux/WebKitGTK-specific**: only there does
the webview fall back to software WebGL. macOS/Windows GPU-accelerate the
existing renderer.

**Observed (`gtk-glarea-stl` feel test):** the wgpu→GtkGLArea path is visibly
smoother than the WebKitGTK renderer **even on an RTX 5070 Ti** — i.e. on a fast
discrete GPU where the GPU is nowhere near the limit. That confirms the Linux
bottleneck is WebKit's *CPU* software-raster, not GPU class: the win from option
B isn't Intel-only, the whole Linux userbase (discrete GPUs included) gets it.

## The decision is binary (you can't mix renderers)

A renderer is the whole scene system — meshes, materials, the spool-color chain,
MMU paint, selection, the gizmo, picking, the bed overlay, *and the separate
G-code preview renderer*. "Keep Three.js on macOS/Windows, use wgpu on Linux"
means writing and maintaining **all of that twice** — once in TS, once in Rust,
kept in sync forever. Nobody does that to fix one platform. So it's one or the
other, everywhere:

- **A) Keep Three.js everywhere.** One render path (JS). Linux stays on software
  WebGL — mitigated by the on-demand rendering already shipped, but the ceiling
  is WebKit's software compositor. macOS/Windows unaffected. **Cheapest; Linux
  doesn't get faster.**
- **B) Go wgpu everywhere.** One render path (Rust); delete Three.js. The *scene
  renderer* is shared across platforms; only a **thin per-platform present shim**
  differs (how the finished frame reaches the screen) — that is NOT a second
  renderer:
  - **Linux:** GtkGLArea (proven). Strategy A's transport (~330 MB/s) is too slow;
    GtkGLArea composites in-process.
  - **macOS:** wgpu can present **directly to a `CAMetalLayer`** (a layer-backed
    NSView) — zero-copy native — or just Strategy A (measured ~5 GB/s, plenty).
  - **Windows:** present a DXGI swapchain on a child HWND / via DirectComposition,
    or Strategy A *if* real-hw transport is fast (the VM said ~95 MB/s, but that's
    a no-GPU VM — unresolved; worst case use the native DXGI/DComp present).
  Best perf everywhere and uniform, but it's the full renderer port (~8–12 wk,
  two renderers if the G-code preview is in scope) **plus** three thin-but-bespoke
  present shims.
- **C) Fix WebKitGTK's GPU path on Linux** (the dmabuf crash) so in-webview WebGL
  is hardware-accelerated like the other two. No port, no second anything — but
  it's a WebKitGTK/driver bug largely outside our control.

Note what's ruled out: **"Strategy A everywhere"** is not viable — shipping frames
to the webview is slow on WebKitGTK (~330 MB/s) and Windows-WebView2 (~95 MB/s,
VM), fast only on macOS. Under option B the present shim is per-platform precisely
because of this (GtkGLArea/CAMetalLayer/DXGI, not a uniform canvas blit).

## Caveats / open items
- **Windows number is from a GPU-less VM** (release build, so not a debug
  artifact, but no GPU). It measured slow *transport*; it did **not** measure the
  thing that matters on Windows (in-webview hardware WebGL), which needs real GPU
  hardware. The reframe above doesn't depend on the VM number — it rests on
  WebView2 being GPU-accelerated on real hardware (well established).
- Linux GtkGLArea still needs its wgpu→GL bridge + transparent-overlay layout if
  pursued (`gtk-glarea/RESULTS.md`).

## Test-setup notes (remote, over SSH)
- **macOS**: headless Metal (wgpu readback) runs over bare SSH. On-screen
  (WKWebView) needs the GUI session — `launchctl asuser` is blocked from SSH, so
  launch a minimal `.app` via `open` (LaunchServices routes it into the logged-in
  session).
- **Windows**: cross-build the `.exe` with `cargo xwin` from Linux. A GUI app run
  directly over SSH lands in a non-interactive window station (WebView2 won't
  init), so run it via a **Scheduled Task** (`LogonType Interactive`) into the
  logged-on session. Custom-scheme URL differs: `http://frame.localhost/` on
  Windows vs `frame://localhost/` on macOS/Linux.
- Both capture results to a file (`/tmp/blit-bench.log`; `C:\tmp\…` on Windows).
