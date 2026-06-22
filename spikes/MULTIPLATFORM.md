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

## Per-platform conclusion

| platform | webview | in-webview WebGL (the *existing* renderer) | needs the wgpu work? |
| --- | --- | --- | --- |
| **macOS** | WKWebView | GPU-accelerated (Metal) → fine | **No** |
| **Windows** | WebView2 | GPU-accelerated (D3D) on real hw → fine | **No** (VM showed slow *transport*, but that's the wrong question — see below) |
| **Linux** | WebKitGTK | **software** (GPU path crashes → `WEBKIT_DISABLE_DMABUF_RENDERER=1`) → slow | **only here** |

## The reframe (the actual conclusion)

This investigation started from a Linux symptom (fans/CPU) and the framing was
"swap the renderer to wgpu." After measuring all three platforms, the cleaner
truth is: **the 3D-perf problem is Linux/WebKitGTK-specific, and macOS/Windows
need no change at all.** Their webviews GPU-accelerate the existing Three.js
WebGL renderer; only WebKitGTK falls back to software.

So the native-GPU work is a **Linux-only** concern, and it's **GtkGLArea**
(proven — `gtk-glarea/RESULTS.md`), not a cross-platform port. macOS and Windows
keep Three.js exactly as-is.

The cost/shape of the decision is therefore:
- **Do nothing more** — keep Three.js everywhere; Linux stays on software WebGL,
  mitigated by the on-demand rendering already shipped. Cheapest.
- **Linux-only native renderer** (wgpu + GtkGLArea) — best Linux perf, but means
  **maintaining a second renderer** just for one platform (Three.js for mac/win,
  wgpu for Linux). Real, ongoing cost.
- **Fix WebKitGTK's GPU path on Linux** (the dmabuf crash) so in-webview WebGL is
  hardware-accelerated like the others — no second renderer, but it's a
  WebKitGTK/driver bug largely outside our control.

"Strategy A everywhere" is **not** the answer: shipping frames to the webview is
slow on both WebKitGTK (~330 MB/s) and Windows-WebView2 (~95 MB/s in-VM); only
macOS is fast. But you'd only ever do that on Linux anyway — and there,
GtkGLArea (in-process GTK composite, no transport) beats it.

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
