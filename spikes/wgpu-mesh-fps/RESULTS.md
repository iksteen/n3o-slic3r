# Will wgpu rendering be faster on Intel? — YES, decisively

The Intel-laptop lag (orbiting a loaded plate) is **WebKitGTK rasterizing the
scene on the CPU** every frame — there is no GPU acceleration for the webview's
WebGL on Linux (the dmabuf GPU path crashes; we ship
`WEBKIT_DISABLE_DMABUF_RENDERER=1`). wgpu moves rasterization onto the iGPU. The
question is whether that's actually fast on Intel hardware.

## Measurement

Headless wgpu render-fps: a lit, depth-tested sphere (stand-in for a print
model), 1080p, camera orbiting, GPU-completion timed per frame. No window, runs
over SSH.

| triangles | **Intel J5005 / UHD 605** (Vulkan, Mesa ANV) | RTX 5070 Ti (sanity) |
| --- | --- | --- |
| 100K (typical model) | **385 fps** (2.6 ms) | 10,781 fps |
| 1M (complex model)  | **195 fps** (5.1 ms) | 8,197 fps |
| 4M (very heavy)     | **61 fps** (16.5 ms) | 3,419 fps |

The J5005's UHD 605 is about the **weakest modern Intel iGPU** — a passively
cooled ~10W Gemini Lake SoC. It still renders:
- a typical 100K-tri model at **385 fps**,
- a complex 1M-tri model at **195 fps**,
- a very heavy 4M-tri scene at **60 fps**.

Any real Intel *laptop* iGPU (Iris Xe / Gen11–12) is several× this.

## Conclusion

GPU rasterization of a representative scene is **not the bottleneck on Intel** —
the iGPU has huge headroom even at the bottom of the range. The current lag is
entirely "the webview rasterizes on the CPU." So **wgpu + GtkGLArea will be, and
feel, faster on Intel** — by a large margin for typical models. The win isn't
marginal; it's CPU-software-raster (laggy) → iGPU-hardware-raster (hundreds of
fps).

## Caveats / what this does and doesn't cover
- This is **render throughput** (the dominant, expensive part), not the full
  interactive loop. Present via GtkGLArea (blit/composite a texture) adds a small
  cost; with 100K–1M models the iGPU has 3–6× headroom over 60fps to absorb it.
- The scene here is a single lit mesh. Real viewport frames add overdraw,
  per-vertex MMU paint colors, selection tint, gizmo, bed grid — minor next to
  triangle rasterization, but not zero.
- Not measured: a direct head-to-head vs WebKitGTK *software* WebGL on the same
  box (would quantify the multiple). The contrast (385 fps vs user-reported lag)
  makes the delta obvious; software raster is typically 10–50× slower than GPU.
- Vulkan worked out of the box (Mesa ANV) on Gemini Lake — no GL fallback needed.

Next, to confirm *feel* (present + vsync + input latency), a small interactive
wgpu→GtkGLArea build on real Intel hardware — but the throughput gate, the thing
that decides the effort, is clearly passed.
