# PR-2-10 — Three.js gizmo (move / rotate / scale)

Status: ❌ open.

**Scope.** The 3-axis transform handle the user drags to translate,
rotate, or scale the selected object(s). Three.js ships
`TransformControls`; this ticket wraps it so drag events round-trip
through PR-2-2's command surface rather than mutating the renderer's
local mirror directly.

The gizmo's *mode* (translate / rotate / scale) is part of the Rust
scene state (`SceneState.gizmo: GizmoState` from PR-2-1). The
frontend reads the mode via `scene:gizmo_changed` and shows the
appropriate handle; user clicks on mode-switching buttons send
`scene_gizmo_set` commands.

**Acceptance criteria.**

- `src/viewport/gizmo.ts`:
  - Wraps Three.js `TransformControls`.
  - On `change` event (drag in progress), updates the local
    object's transform for immediate visual feedback.
  - On `mouseUp` / `dragging-changed=false`, sends the *final*
    transform to Rust via `scene_object_set_transform`. The Rust
    side validates + emits `scene:object_updated`; the renderer's
    event handler applies the canonical transform (which may
    differ from the local-feedback transform if Rust applied any
    constraints).

- Gizmo respects the `pivot` field of `GizmoState`:
  - `Default` — pivot at object center.
  - `Origin` — pivot at world origin.
  - `Custom(Vec3)` — user-supplied pivot (Phase 4 UI).

- Snap support:
  - Translate snap: 1 mm increments (configurable, but ship one
    sensible default).
  - Rotate snap: 15° increments.
  - Scale snap: disabled by default (continuous).
  - Toggled by holding Shift during drag (Three.js
    TransformControls native).

- Multi-select transform: when the gizmo is active and multiple
  objects are selected, the drag affects all of them. Rust side
  applies the same delta transform to each via a sequence of
  `scene_object_translate` / `_rotate` / `_scale` commands.

- Tests: JS-side `__test__/gizmo.test.ts` simulates a translate
  drag and asserts the emitted command sequence ends with one
  `scene_object_set_transform` per selected object, with the
  expected final transform.

**Effort.** ~2 days.

**Dependencies.** PR-2-9 (renderer + selection visualization),
PR-2-5 (transform commands).

**Out of scope.** Custom gizmo handles (e.g. uniform-scale-only
handle) — Three.js TransformControls' defaults suffice for MVP.
Numerical input boxes (type a precise value) — Phase 4 UI work.
Constraint-aware drag (snap to other objects) — Phase 4+.
