# Strategy B PoC — native wgpu surface in a Tauri 2 window

**Question:** can wgpu render to a *native* Tauri window surface (via
`window_handle()`) and coexist with a webview in the **same** window — avoiding
Strategy A's offscreen-render + pixel-copy?

**Method:** minimal Tauri 2 app (`unstable` feature). Build a bare `Window`
(no default webview), create a wgpu surface on its `window_handle()`, clear it
**magenta**, then `add_child` a webview (green panel) over the right half. If we
see magenta-left + green-right, stable, Strategy B coexists. Run on this
WebKitGTK/Wayland machine, both Wayland and XWayland backends.

## Result: does NOT work on Linux — two distinct failure modes

**Wayland (`WAYLAND_DISPLAY`, default):**
```
[poc] window handle: Wayland(WaylandWindowHandle { surface: 0x... })
[poc] adapter: NVIDIA GeForce RTX 5070 Ti (Vulkan)
[poc] presented magenta frame            <- wgpu surface creation + present SUCCEEDS
[poc] add_child webview (right half) OK
...
Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display.   <- CRASH
```
wgpu takes the `wl_surface` and presents fine; the moment the GTK/WebKit webview
also renders to that same surface it's a Wayland protocol violation (two clients
on one surface) → fatal `Gdk Error 71`.

**XWayland (`GDK_BACKEND=x11`):**
- No crash. `window handle: Xlib(...)`, wgpu presents magenta, app stays up.
- But the screenshot is **100% magenta — the webview is invisible.** wgpu's
  swapchain presents to the whole X11 window, painting over the webview, which
  GTK draws *client-side into the same X window*. (`poc-x11-shot.png`.)

## Root cause

GTK renders the webview into the **same single OS window surface** that wgpu's
swapchain presents to. They can't share one surface: on Wayland that's a
protocol error (crash); on X11 wgpu's present overwrites GTK's drawing (webview
occluded). This is the "GTK and wgpu are different rendering systems" wall the
ecosystem documents (no Tauri maintainer guidance; the canonical
`wgpu-tauri-experiment` is non-functional + macOS-only).

There is no Linux escape via the multi-webview "native region" trick either: it
works for GStreamer because GStreamer integrates as a **GTK widget**
(GtkGLArea / dmabuf sink) inside GTK's compositing. wgpu has no GTK-widget
integration — to feed a GtkGLArea you'd render wgpu **offscreen and upload the
texture**, which is Strategy A again, just presented through a GTK widget
instead of an HTML `<canvas>`.

## Conclusion

- **Strategy B is not viable on Linux** (our primary / flatpak target). Confirmed
  empirically here, matching the literature.
- It *may* work on macOS/Windows (separate native child `NSView`/`HWND` layers),
  but those aren't the constraint — Linux is.
- **Strategy A (offscreen wgpu → present to a webview canvas, or to a GtkGLArea
  widget) is the only native-GPU path on Linux.** Spike 0 already showed the
  wgpu render + readback half is cheap (~1.5ms @4K); the remaining make-or-break
  is the transfer/composite into the webview.

So: keep the GPU rendering native (wgpu, the big win over WebKit software-GL),
but the bridge to the UI must be offscreen→copy on Linux — there is no
single-surface native-window composite with a GTK webview.
