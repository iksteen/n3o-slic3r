# PR-2-9 — Three.js renderer (scene + orbit + camera modes)

Status: ✅ shipped.

**Scope.** The frontend viewport: Three.js scene that subscribes to
PR-2-2's events and mirrors the authoritative Rust state. Camera
with orbit controls, perspective + orthographic projection toggle,
zoom-to-fit, framing helpers.

Per the AD-8 invariant: the renderer holds *no* authoritative state.
It maintains a local mirror keyed by `ObjectId` (Rust's side) and
re-applies every event. On reconnect / restart it calls
`scene_snapshot()` to rebuild the mirror from scratch.

**Acceptance criteria.**

- `src/viewport/` directory (new):
  - `ViewportCanvas.tsx` — root React component rendering a
    Three.js `<canvas>` via a hand-written `useEffect`-driven
    Three.js boot.
  - `sceneMirror.ts` — local Three.js `Group` containing per-object
    `Mesh`es indexed by `ObjectId`. Pure data structure.
  - `eventBridge.ts` — Tauri event subscribers calling
    `sceneMirror.applyEvent(event)` per emission.

- Event-application rules:
  - `scene:mesh_loaded` → register vertex buffer
    (`Three.BufferGeometry` from the event's vertex/index arrays).
  - `scene:object_added` → instantiate `Mesh` with the appropriate
    geometry + material, apply the event's `transform` matrix.
  - `scene:object_updated` → look up by `id`, apply the new
    `transform` / name / visibility.
  - `scene:object_removed` → dispose `Three.Mesh.geometry +
    .material`, remove from group.
  - `scene:selection_changed` → re-color selected objects
    (outline shader for MVP).
  - `scene:camera_changed` → reset camera to the event's state.
  - `scene:bed_changed` (from PR-2-6) → redraw bed mesh +
    exclusion zone wireframes.

- Camera controls:
  - Orbit (left-drag rotates around target).
  - Pan (middle-drag).
  - Zoom (scroll).
  - Perspective ↔ orthographic toggle. **Ortho is a cut candidate**
    per the Execution Plan.
  - `Frame All` button → adjusts camera so all visible objects fit
    in the viewport.

- User-intent flows back through commands:
  - Drag with no gizmo → camera orbit only (no state mutation).
  - Click on object → `scene_select` with the clicked object.
  - Pressing `Delete` with selection → `scene_object_delete`.

- Tests at the renderer level are JS-side and primarily visual.
  Programmatic test: a stub viewer in `src/viewport/__test__/`
  that logs events to a buffer; sequence "load → select → translate
  → deselect" produces the expected log lines without rendering
  actual graphics.

**Effort.** ~5 days. Three.js scene plumbing + event bridge is the
bulk; orbit controls + framing are mechanical.

**Dependencies.** PR-2-1 + PR-2-2 (Rust side; the renderer
subscribes to those events). PR-2-3 (so there's something to load
and see).

**Out of scope.** Gizmo handles (PR-2-10). Performance optimization
beyond "works" — that's PR-2-11's job. Specific shaders for
multi-color paint preview — Phase 5 / Phase 7 work. wgpu pivot —
PR-2-11 decides; if pivot, all of this renderer code is replaced
not extended.

**Cut candidate.** Orthographic camera toggle (~1 day savings).
