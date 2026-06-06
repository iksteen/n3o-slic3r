# PR-5-6 — Model material → slot binding model

Status: ❌ open.

**Scope.** First-class storage + validation surface for the
per-(plate, printer) mapping from model material indices
(1..N, as referenced by per-volume `extruder` metadata) to
physical slots on the assigned printer's AMS / toolchanger.
Live slot polling (printer-state-driven availability) is
**stubbed** in Phase 5; PR-7c wires the real polling.

Owns FR-MP-8 (foundations) + FR-FS-6.

**Acceptance criteria.**

- `MaterialBinding` already declared in PR-5-1; this ticket
  surfaces it via Tauri commands + a validation pass.

- New commands:
  - `project_set_material_binding(plate_id, model_material, physical_slot, filament_identity)` —
    upserts; emits `scene:material_binding_changed { plate_id }`.
  - `project_clear_material_binding(plate_id, model_material)` —
    drops the binding; the model material falls back to
    "use slot 1" at slice time.
  - `project_auto_bind_materials(plate_id) -> Vec<MaterialBinding>`
    (FR-FS-10): apply the auto-binding heuristic — for each
    model material referenced by an object on the plate,
    match against the printer's loaded filaments by family
    (PLA → first PLA slot, PETG → first PETG slot, etc.).
    Returns the proposed bindings; user confirms / adjusts
    via the binding UI.

- Validation pass at cascade-resolution time (FR-MP-8):
  - Walk the plate's `material_bindings`.
  - For each binding: verify `physical_slot` is within
    `printer.slot_count` and is "available" (Phase 5 stubs
    this to always-true; Phase 7c reads real slot state).
  - For each model material referenced by an object on the
    plate: verify a binding exists. Missing bindings
    surface as `slice_blocker` errors.

- Pre-slice gate: extend PR-3-2's `start_slice_job` to
  call `validate_material_bindings(&plate)` and refuse
  to start the job if any model material is unbound or
  bound to an unavailable slot. Error message includes
  the model material index + a one-click "auto-bind"
  suggestion (FR-FS-10 redux).

- New `src/plates/MaterialBindingPanel.tsx`:
  - Lists each model material referenced by objects on the
    active plate.
  - Per row: dropdown for physical slot (1..N for the
    bound printer) + filament-profile picker.
  - "Auto-bind" button that calls
    `project_auto_bind_materials` and pre-fills.
  - Warning indicator on unbound rows.

- Tests:
  - 4-color fourcolor.3mf fixture: assign A1 mini, auto-
    bind, verify 4 bindings produced matching the printer's
    4 AMS slots.
  - Slice attempt with one model material unbound: errors
    before launching the worker thread.
  - Slice attempt with an out-of-range slot binding:
    same.

**Effort.** ~3 days. The auto-binding heuristic + the
validation pass + the binding panel UI is the bulk; the
data structures + commands are mechanical from PR-5-1.

**Dependencies.** PR-5-1 (`MaterialBinding`), PR-5-2
(per-plate scene state with object metadata for the
material-index walk), PR-3-2 (`start_slice_job` gate).

**Out of scope.** Live slot polling from connected
printers (Phase 7c). Multi-color paint UI for assigning
paint regions to materials (Phase 7c — FR-FS-13).
Sync-on-send metadata emission into the `.3mf` send
format (Phase 7c — FR-FS-11). Filament family
auto-detection beyond the literal `base_type` string
match (Phase 9 polish).

**Cut candidate.** The auto-bind heuristic (~1 day) —
ship the panel with manual binding only. The PRD
requirement (FR-FS-10) becomes Phase 9.
