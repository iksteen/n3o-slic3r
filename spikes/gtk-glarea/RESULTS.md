# Spike: GtkGLArea coexists with the webview in one Tauri window — IT WORKS

**Question:** can native GPU content sit alongside the WebKitWebView in one
Tauri window on Linux/Wayland — given Strategy B's raw wgpu swapchain crashed
(`Gdk Error 71`) and Strategy A's webview transport is capped at ~330 MB/s?

**Method:** Tauri 2 app. In `setup`, reach into Tauri's Linux GTK window via
`win.default_vbox()` (the `gtk::Box` Tauri packs the webview into) and
`pack_start` a `gtk::GLArea` as a sibling band on top. The GLArea's `render`
signal clears it **orange** via GL (epoxy). The webview shows a **green** page.

## Result: YES — orange GL + green webview, one window, Wayland, no crash

`glarea-coexist-shot.png`: orange GL band on top, green WebKitGTK webview below,
in a single Tauri window. The app stays alive; the GLArea `render` fires
continuously. Logs:

```
[poc] GtkGLArea packed into Tauri's default_vbox
[poc] GLArea render fired (orange clear)   (x20+, no crash)
```

Run on the same WebKitGTK/Wayland machine where Strategy B crashed. **No
`Gdk Error 71`.** The reason: a `GtkGLArea` is a *GTK widget* composited by GTK
into the window — not a raw Vulkan swapchain fighting GTK for the `wl_surface`.
GTK owns the surface and lays out the GL widget + the webview widget together.

## Why this matters

This is the **native-GPU-on-Linux path that beats both walls**:
- vs **Strategy B** (raw swapchain): no surface conflict / crash — GTK composites
  the GL widget.
- vs **Strategy A** (offscreen → webview): no ~330 MB/s webview transport — the
  GL content is presented in-process by GTK. Getting a wgpu frame onto the
  GLArea is either a GPU-side GL-context share / DMA-BUF import (zero-copy) or,
  as a safe fallback, wgpu readback (~1.5ms, Spike 0) + `glTexSubImage2D` upload
  (a GPU DMA at multi-GB/s, not 330 MB/s) + a textured quad.

## What's proven vs. what's left

**Proven here:** GTK-composited GPU content + the webview coexist in one Tauri
window on Wayland. The `default_vbox()` GTK-surgery hook works. No crash.

**Remaining engineering (next spikes), in order of risk:**
1. **wgpu → GLArea bridge.** Easiest: wgpu offscreen + readback + `glTexSubImage2D`
   into a GLArea texture + draw a quad in the `render` callback. Harder/ideal:
   share the GLArea's GL context with wgpu's GL (gles) backend, or Vulkan→GL
   DMA-BUF import — zero-copy, true native.
2. **Hole-punch layout for the real UI.** The viewport isn't a top band — it's a
   central region with React panels around it. Cleanest: a `GtkOverlay` with the
   GLArea as the base (full window) and a **transparent** WebKitWebView on top
   whose viewport region is a transparent `<canvas>`-less hole, so the GLArea
   shows through. Input stays in JS (as today) — the webview is on top and owns
   pointer events; it forwards camera/pick intents to Rust. Unknown to verify:
   transparent WebKitGTK webview over a GL widget on Wayland.
3. The render loop: drive `GLArea::queue_render()` on `invalidate` (the on-demand
   model already in the app), not every frame.

## Verdict

The architecture is viable on Linux: **viewport = GtkGLArea (wgpu-fed), UI =
transparent webview on top.** It's more GTK/interop plumbing than Strategy A, but
it's the only path that is both crash-free AND free of the transport wall — i.e.
the only one that can give true native 60fps on Linux. Worth pursuing if the
wgpu port goes ahead.
