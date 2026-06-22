# Spike 0 results — Strategy A (offscreen wgpu → blit to webview canvas)

Strategy A's per-frame cost has two halves:
1. **Rust half** — wgpu render + GPU→CPU readback. (this spike)
2. **Webview half** — IPC to JS + `canvas.putImageData` + WebKit *software* composite. (pending)

Budget: ~16.7 ms = one 60fps frame. Strategy A only has to beat WebKit's
software *triangle rasterization* (today's Linux path), but 60fps is the bar.

## Half 1 — wgpu render + readback: decisive PASS

`cargo run --release` (standalone crate, no libslic3r). On the dev desktop:

```
adapter: NVIDIA GeForce RTX 5070 Ti | backend Vulkan | type DiscreteGpu
res        MB/frame  encode ms  render+readback ms   readback MB/s  60fps?
1080p        7.9 MB     0.03              0.46             17193    PASS
4K          31.6 MB     0.03              1.56             20256    PASS
```

- 4K readback costs **~1.5 ms** — ~10× under the 60fps budget.
- Crucially this is **resolution-bound, not scene-bound**: it stays ~1.5ms no
  matter how heavy the scene (it's a fixed-size texture copy). The scene-cost
  lives in GPU rasterization, which is exactly what we're moving onto the GPU.
- This was the number that could have killed Strategy A on the GPU side. It
  didn't, with an order of magnitude to spare.

**Caveat — integrated GPU not yet measured.** The Intel laptop (unified memory)
needs its own run. Readback is often *fine* on integrated (no PCIe transfer),
but confirm before relying on it.

## Half 2 — webview transfer + composite: measured-pending

Not yet measured. Attempted via `webkit2gtk-4.1 MiniBrowser` (the exact engine
the app links, `libwebkit2gtk-4.1.so.0`) but it's not a trustworthy harness:
- no console→stdout forwarding to extract results,
- rAF background-throttles when the window isn't focused/visible (ruins cadence),
- and the sync `putImageData` time excludes the *composite/present* cost, which
  is the uncertain software-path part.

An honest measurement needs a **focused, visible** WebKitGTK webview presenting a
4K frame every vsync and reporting achieved fps — i.e. a minimal Tauri/WebKit
harness (~half a day), not a headless browser.

Reasoned bound (not measured): `putImageData` of a 4K RGBA frame (~33 MB) is a
memcpy into the canvas backing (~3–4 ms at memory bandwidth) plus WebKit's
software composite of that canvas (~another few ms) → order **~5–10 ms at 4K**,
**~1–2 ms at 1080p**. Within the 16.7ms budget, but it eats a real slice at 4K.
This must be measured before committing.

## Verdict so far

The GPU/Rust half is a clear PASS. The make-or-break is now entirely the webview
transfer+composite half. Next step: a minimal visible WebKit harness to get the
4K end-to-end fps number, and a run of this spike on the Intel laptop.
