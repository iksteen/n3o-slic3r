# PR-7c-8 — Multi-color paint UI binds to material indices

Status: ❌ open.

**Scope.** When the user paints regions of a model with multiple
colors, those paint regions assign to **model material indices**
(1..N) — never directly to physical slots. The binding layer
(PR-5-6 + PR-7c-6) always mediates between paint index and
physical slot.

If no paint UI exists yet (the existing scene loader treats
each object as a single material), this ticket adds the
minimum-viable paint surface.

**Acceptance criteria.**

- **Audit existing paint surface**:
  - Check whether Phase 2's mesh loader / Phase 5's scene
    state already supports per-vertex material indices. If
    yes, this ticket is wiring-only.
  - If no, add `Mesh.per_face_material: Vec<u8>` (one
    material index per triangle), default all-zeros.

- **Paint UI** (component scope to be decided per audit):
  - "Paint Material" mode toggle in the viewport toolbar.
  - When active, mouse hover shows a paint brush; click
    paints the hit triangle with the currently-selected
    material index (1..N picker).
  - Paint is stored on the Mesh; survives scene save/load
    (PR-5-8 extension).

- **Material-index → slot indirection is invariant**:
  - The orchestrator's input builder (PR-6-1) reads
    `Plate.material_bindings[printer_identity]` and emits
    per-face `extruder` metadata in the temp `.3mf`.
  - Per-face extruder assignment respects the binding —
    not the paint index directly. So material index 3 paint
    + binding `3 → slot 2` = extruder 2 in the slice output.

- **Pre-slice gate**: refuse to slice if a painted material
  index has no binding (`MaterialBinding` missing for that
  index). Surfaces in the MaterialBindingPanel as "Material 3
  is painted but not bound."

- Tests:
  - **`per_face_material_survives_3mf_save_load`** —
    paint a mesh, save, reload, assert the same painted
    face has the same material index.
  - **`orchestrator_resolves_paint_through_binding`** —
    paint material 3 + binding `3 → slot 2`, slice, assert
    the resulting .3mf temp file has extruder 2 on the
    painted faces.
  - **`pre_slice_gate_blocks_painted_but_unbound_material`** —
    paint material 3 with no binding, slice attempts to
    start, gets refused.

**Effort.** ~2 days if no paint UI exists; ~1 day if wiring-
only. Audit drives the estimate.

**Dependencies.** PR-5-6 (bindings), PR-5-8 (project save),
PR-6-1 (orchestrator input builder).

**Out of scope.**

- Paint stroke smoothing / brush sizes / paint UNDO — MVP
  paint is a click-per-triangle toggle. Polish in Phase 9.
- Paint via mesh slicing planes (paint by Z range) — post-
  MVP.
- Auto-segmentation of imported multi-color 3MFs into paint
  regions — out of scope (the 3MF's per-vertex color is
  Phase 2 import territory).
