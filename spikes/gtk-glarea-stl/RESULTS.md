# wgpu → GtkGLArea, interactive — the feel-test harness

A standalone GTK3 app that renders an STL on a build plate and lets you orbit
(left-drag) and zoom (scroll). This is the **production Linux present path** —
wgpu on the iGPU, composited by GTK — minus the webview (coexistence with the
WebKitWebView is already proven in `../gtk-glarea`). It exists to answer the last
open question from the throughput spike: does it *feel* smooth on Intel?

## Run

```bash
cargo run --release -- /path/to/model.stl
# default model: ../../../ocon/m5sticks3_click_case_no_logo.stl
```

Left-drag orbits, scroll zooms. Needs a real GL/Wayland (or X) session — i.e. run
it on the box you want to feel, not over bare SSH.

## The bridge (the previously-unbuilt piece)

Zero-copy, no readback:

1. **Adopt the GLArea's own GL context.** In `connect_render` GTK has made its
   GdkGLContext current; `wgpu_hal::gles::Adapter::new_external(loader)` builds a
   wgpu adapter on *that* context. Loader = `epoxy::get_proc_addr` (libepoxy
   exports `epoxy_*` dispatch pointers, not plain `glFoo` — a raw dlsym finds
   nothing; this was the first wall).
2. **Render the scene into an offscreen wgpu texture** (lit, depth-tested; same
   pipeline as the mesh-fps spike, plus a build-plate line grid).
3. **`glBlitFramebuffer` the texture into GTK's framebuffer.** Capture GTK's FBO
   id at the *top* of the frame — wgpu's submit rebinds its own FBO on the shared
   context and never restores GTK's (the second wall: blitting to the
   wgpu-left-bound FBO → black window). Y-flipped (wgpu top-left → GL
   bottom-left). The texture never leaves the GPU.

`N3O_DUMP=1` writes the offscreen texture to `/tmp/wgpu-dump.ppm` once — the
debug check that isolated "wgpu rendered fine" from "the blit was wrong".

## Status

Built + driven headless (Xvfb, xdotool): renders correctly, orbit + zoom work,
no crashes (validated on the RTX dev box via the GL backend). The bridge is the
real one the production renderer would use. **Remaining: run it on the Intel box
and feel the orbit** — the throughput gate (`../wgpu-mesh-fps/RESULTS.md`) is
already passed; this confirms present + input latency.
