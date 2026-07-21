# Browser demo of the n3o UI

A static, self-contained build of the real n3o frontend that runs in any
browser with a **mocked backend** — so people can get a feel for the app
without installing it. The sample project is the OrangeCon M5 StickS3 case on an
AMS-equipped A1 mini, sliced by n3o.

It's a *feel* demo, not a working slicer: the backend is canned, there's no real
slicing (a pre-baked result is shown), and Devices / Send / settings-editing are
inert chrome.

## Build

```bash
npm run demo:build
```

Outputs into `demo/dist/` (git-ignored):

- `dist/app/` — multi-file bundle. **Upload this folder** to a static host.
- `dist/app.single.html` — the same thing inlined into one file. Drop it
  anywhere (works from a subpath or `file://`).
- `dist/app.artifact.html` — one-file build with the `<html>/<head>/<body>`
  wrapper stripped, for publishing as a claude.ai Artifact.

## How it works

`vite build --mode demo` (see `vite.config.ts`) turns the normal frontend into
the demo — the regular app build is untouched:

- A plugin redirects the two native (Rust wgpu) 3D surfaces to browser WebGL
  renderers: `viewport/WgpuViewport` → `demo/mock/DemoViewport.tsx` (lit model
  on a build plate), `GcodePreview` → `demo/mock/DemoPreview.tsx` (feature-
  colored toolpath tubes, layer-window aware).
- `@tauri-apps/api/{core,event,webview}` are aliased to shims in `demo/mock/`.
  `commands.ts` holds the canned `invoke` responses (scene snapshot, printer
  instance + AMS, real exported settings, and the simulated Slice → preview
  event flow).
- `inline.py` inlines the JS/CSS into one file, inlines the logo as a data URI
  (its absolute `/brand-icon.svg` path 404s at a subpath), and pins the app to a
  consistent dark theme before mount (the artifact wrapper's `data-theme`
  otherwise fights the app's own and leaves it half-themed).

## Regenerating the data assets

`demo/assets/*.json` are committed (a fresh checkout can build without a slice),
but they're generated from the sample model + real n3o slices/introspection.
`sample.gcode` is a git-ignored intermediate. To regenerate (needs a built
libslic3r FFI — `N3O_SLIC3R_FFI_CMAKE_CONFIG=Release`):

```bash
# mesh for the model viewer (from the source .3mf, through n3o's importer)
cargo run -p n3o-slic3r --example dump_mesh -- demo/assets/model.3mf demo/assets/mesh.json

# toolpaths for the preview: slice the model, then extract per-layer polylines
cargo run -p n3o-slic3r --features test-fixtures --example slice_repro -- demo/assets/model.3mf
cp <output-dir>/plate_1.gcode demo/assets/sample.gcode
python3 demo/extract_toolpaths.py demo/assets/sample.gcode demo/assets/toolpaths.json

# the settings panels: real option summaries + resolved cascade for the A1 mini
cargo run -p n3o-slic3r --features test-fixtures --example dump_settings -- demo/assets
```

To use a different sample model, swap `demo/assets/model.3mf` and rerun the
above.
