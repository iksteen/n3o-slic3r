# wgpu renderer — multi-platform picture (measured)

The native-window path (GtkGLArea) is Linux-only and doesn't port — GTK doesn't
exist on macOS/Windows. The question was whether that's a problem. **It mostly
isn't**, because the thing that *motivated* the native path — the ~330 MB/s
frame-transport wall — is **WebKitGTK-specific**.

## Strategy A (offscreen wgpu → present to webview canvas), measured

Same benchmark crate (`webview-blit`), same code, two platforms:

| stage | Linux WebKitGTK (RTX 5070 Ti) | macOS WKWebView (Apple M4) |
| --- | --- | --- |
| wgpu render + readback @4K | ~1.56 ms (Vulkan) | ~1.34 ms (Metal, unified mem) |
| `putImageData` @4K | 2–3 ms | 3 ms |
| present cadence (all res) | 60 fps (vsync) | 60 fps (vsync) |
| **transport, custom protocol @4K** | 93 ms (**~340 MB/s**) | 6.5 ms (**~4.9 GB/s**) |
| **transport, custom protocol @1080p** | 25 ms (~313 MB/s) | 1.6 ms (~4.9 GB/s) |

**macOS transfers frames ~15× faster than WebKitGTK.** End-to-end on macOS @4K:
readback 1.3 + transport 6.5 + putImageData 3 ≈ **11 ms < 16.7 ms → 60 fps via
plain Strategy A.** No native compositing required.

## Per-platform conclusion

| platform | webview | Strategy A verdict | native path needed? |
| --- | --- | --- | --- |
| **macOS** | WKWebView | ✅ 60fps @4K (measured) | **No** |
| **Linux** | WebKitGTK | 🟡 transport-bound (~330 MB/s); fine at small viewports, drops at large | only if large-viewport orbit matters → **GtkGLArea** (proven, Linux-only) |
| **Windows** | WebView2 | ❓ untested — but WebView2 is hardware-composited with a modern IPC; **expected like macOS** | probably No (verify) |

## The architecture this points to

**Strategy A as the uniform, cross-platform baseline** — one codepath, works on
all three, the only one shippable+testable uniformly. It's plenty on macOS
(measured) and almost certainly on Windows.

**GtkGLArea is not "the cross-platform answer" — it's the Linux-only escape
hatch**, applied behind a `cfg(target_os = "linux")` *iff* WebKitGTK's transport
actually hurts (i.e. large-window orbit). macOS/Windows never touch it.

So the multi-platform cost of going native is **not** three bespoke integrations.
It's: Strategy A everywhere + one Linux-only GtkGLArea optimization. The
NSView/CAMetalLayer (macOS) and child-HWND/DComp (Windows) native paths — which
*would* be bespoke — are **unnecessary**, because those webviews are fast enough
for Strategy A.

## Open items
- **Windows**: measure the same `webview-blit` bench on WebView2 to confirm it's
  in the macOS class (fast) and not the WebKitGTK class (slow). Needs Windows
  hardware/CI. Until then it's an assumption.
- The Linux GtkGLArea path still needs its wgpu→GL bridge + transparent-overlay
  layout if pursued (see `gtk-glarea/RESULTS.md`).

## Test setup note (macOS over SSH)
Headless Metal (wgpu offscreen render + readback) runs over a bare SSH session.
On-screen tests (WKWebView) need the active GUI login session: `launchctl asuser`
is blocked from SSH (audit-session switch needs root), so launch a minimal `.app`
bundle with `open` (LaunchServices routes it into the logged-in session); the
bench reports results to `/tmp/blit-bench.log` for capture.
