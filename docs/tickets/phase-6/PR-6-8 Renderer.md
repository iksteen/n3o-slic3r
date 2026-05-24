# PR-6-8 — `GcodePreview` Three.js renderer

Status: ✅ shipped.

**Scope.** Top-level React component that hosts the preview's
Three.js scene. Owns its own renderer, camera, controls, and
mesh tree — parallel to the viewport's `ViewportCanvas`, not
nested in it. Mounts when the App is in preview mode (PR-6-15).

**Acceptance criteria.**

- New module `src/preview/`. Suggested layout:
  ```
  src/preview/
    GcodePreview.tsx       # React component owning the canvas
    previewScene.ts        # Three.js scene + camera + lights
    geometryBuilder.ts     # BufferGeometry from preview_buffers
    shaderMaterial.ts      # ShaderMaterial with layer-cull uniforms
    bedGrid.ts             # Bed grid shared with viewport (see notes)
    types.ts               # TS mirrors of the Rust SegmentDetail etc.
    invokes.ts             # Tauri invoke wrappers for PR-6-7's commands
  ```

- **`<GcodePreview/>` props:**
  ```tsx
  interface GcodePreviewProps {
    handle: PreviewHandle | null;  // null = empty state
    bedExtents: BoundingBox | null;  // for bed grid sizing
    colorMode: ColorMode;
    palette: Palette;
    layerWindow: LayerWindow;  // see PR-6-9
    showTravels: boolean;
    showRetractions: boolean;
    onSegmentHover: (detail: SegmentDetail | null) => void;
    onLayerCount: (count: number) => void;  // bubble up for slider
  }
  ```

- **Scene composition:**
  - Single `OrbitControls` camera, default position `(bed_cx,
    bed_cy - bed_depth * 0.5, max_z * 1.5)` looking at the
    print's center.
  - Bed grid (see "Bed grid sharing" below) — same colors +
    line style as the viewport.
  - Two `LineSegments` objects: `extrusionsMesh` + `travelsMesh`.
  - One `Points` object for retractions (small red dots at
    retract positions).

- **Geometry build pipeline:**
  - On `handle` change: invoke `preview_buffers(handle,
    colorMode, palette)`, parse the binary blob, construct
    `BufferGeometry` with attributes `position`, `color`,
    `aLayer`.
  - On `colorMode` / `palette` change: invoke
    `preview_buffers` again, swap only the `color` attribute
    (geometry + `aLayer` survive).
  - On `handle = null`: dispose all GPU buffers.

- **Shader material** (`shaderMaterial.ts`):
  ```glsl
  // vertex shader
  attribute float aLayer;
  attribute vec3 color;
  varying vec3 vColor;
  varying float vLayer;
  void main() {
    vColor = color;
    vLayer = aLayer;
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }

  // fragment shader
  uniform float uLayerMin;
  uniform float uLayerMax;
  varying vec3 vColor;
  varying float vLayer;
  void main() {
    if (vLayer < uLayerMin || vLayer > uLayerMax) discard;
    gl_FragColor = vec4(vColor, 1.0);
  }
  ```

  `layerWindow` updates patch `uLayerMin` / `uLayerMax`
  directly — no buffer rebuild.

- **Bed grid sharing.** The Phase 2 bed-grid lives inside
  `ViewportCanvas.tsx`. Two options:
  - **Extract** to `src/viewport/bedGrid.ts` (~50 lines of
    Three.js) and import from both `ViewportCanvas` and
    `previewScene.ts`.
  - **Reimplement** in `preview/bedGrid.ts`.

  Decide during impl based on diff size. The extract path is
  cleaner but requires touching Phase 2 code; reimplementation
  keeps the preview self-contained. Recommendation: extract.

- **Camera persistence:** the preview's camera state is
  **not** persisted across mode toggles in the MVP. Every
  preview-mount lands at the default view. Persistent
  camera is a Phase 9 polish.

- Tests (`src/preview/__test__/`):
  - **`geometryBuilder` parses the binary blob correctly:**
    feed a known short blob, assert positions + colors +
    layer indices come out as expected.
  - **`shaderMaterial` clamps layerWindow to valid range:**
    `layerWindow.max > layerCount` → clamps to layerCount.
  - **DOM smoke** (needs jsdom; defer if the testing
    harness isn't ready — note in the ticket if so).

**Effort.** ~3 days. The single largest ticket in Phase 6.
Most of the time goes into the geometry-builder + shader
material + camera defaults + integration with the rest of
the preview surface.

**Dependencies.** PR-6-7 (Tauri commands the renderer
invokes), Three.js (already in package.json from Phase 2).

**Out of scope.**

- Layer slider UI (PR-6-9 owns the slider; this ticket
  accepts `layerWindow` as a prop and shaders accordingly).
- Travel / retraction toggle UI (PR-6-10; same pattern as
  layerWindow — accept as props).
- Hover tooltip UI (PR-6-11; this ticket calls
  `onSegmentHover` on raycast hit, but the tooltip DOM is
  in PR-6-11).
- Color picker UI (PR-6-13).
- Stats panels (PR-6-12).
- App mode toggle (PR-6-15).

**Cut candidate.** None — central to the phase.
