# Strategy A — webview-half benchmark (the transfer + composite cost)

Strategy A = render with wgpu natively (Spike 0: ~1.5ms @4K, scene-independent),
then get the frame into the webview and on screen. This measures that second
half on the **WebKitGTK software path** (`WEBKIT_DISABLE_DMABUF_RENDERER=1`, the
realistic flatpak case). JS reports each result back through a `report` Tauri
command (reliable capture). Machine: RTX 5070 Ti, 3440×1440 screen, dpr=1.

> Caveat: the bench window was tiled by the WM (not the requested size), so the
> *composite* numbers reflect a real on-screen window ≤ screen res — which is the
> realistic range anyway. The *transport* numbers are IPC throughput and
> window-size-independent.

## Results

| stage | 1080p | 1440p | 4K |
| --- | --- | --- | --- |
| `putImageData` (data copy into canvas backing) | ~0 ms | 1 ms | 2–3 ms |
| present cadence (putImageData every rAF + composite) | 16ms / **63fps** | 16ms / **63fps** | 16ms / **63fps** |
| transport via `invoke` channel | 30 ms (266 MB/s) | — | 126 ms (251 MB/s) |
| transport via custom URI scheme (fetch, pre-alloc'd) | 25 ms (**313 MB/s**) | — | 93 ms (**339 MB/s**) |

## Reading the numbers

- **Canvas + WebKit software composite is a non-issue.** Presenting a fresh frame
  every vsync sustains 60fps *even at 4K* — the blit+composite has headroom under
  16.7ms. `putImageData` itself is ≤3ms.
- **The transport is the wall: ~330 MB/s**, and it's *not* `invoke`-specific — the
  custom URI scheme (Tauri's asset path) is the same ceiling. ~330 MB/s for what
  should be memory copies is ~20× slower than memcpy, so WebKitGTK's resource
  pipeline is doing real per-byte work (chunked soup buffers / multiple copies),
  not just bandwidth. This is a WebKitGTK-internal cost.

What ~330 MB/s implies for **interaction** framerate (frames that change — orbit/
drag; static scenes transfer nothing thanks to on-demand rendering):

| viewport blit size | bytes | transport | ~max fps |
| --- | --- | --- | --- |
| 1280×800 (small window viewport) | 4 MB | ~12 ms | ~80 fps ✅ |
| 1920×1080 | 8 MB | ~25 ms | ~40 fps 🟡 |
| 2400×1300 (maximized ultrawide viewport) | 12 MB | ~36 ms | ~28 fps 🟠 |
| 3840×2160 | 32 MB | ~93 ms | ~11 fps ❌ |

## Verdict for Strategy A

Strategy A **works**, and the GPU rendering is genuinely native (the win over
WebKit software-GL). But the Rust→webview **transport (~330 MB/s) caps
interaction framerate**, scaling with viewport pixels: smooth (~60–80fps) at
small viewports, marginal (~40fps) at 1080p, and poor (~11fps) at 4K. The
bottleneck is neither the GPU (1.5ms) nor the composite (60fps) — it's getting
the bytes into the webview. On-demand rendering hides this for static scenes;
the cost is paid during orbit/drag, worst at large window sizes.

### Faster transports not yet measured (would change the verdict)
- **GtkGLArea (the real fix):** render wgpu to a GL texture and display it in a
  `GtkGLArea` widget composited by GTK alongside the webview — **GPU-side, zero
  readback, zero transfer**, sidestepping both the 330 MB/s wall *and* the
  Strategy-B surface conflict (it's a proper GTK widget, not a raw swapchain).
  Cost: reaching into Tauri's GTK window to add a widget + wgpu↔GL interop —
  hacky/fragile, a separate spike. This is the path most likely to give true
  60fps natively on Linux.
- Persistent WebSocket / SharedArrayBuffer (COOP/COEP) — may amortize per-fetch
  overhead, but likely still copy-bound at a similar ceiling.
