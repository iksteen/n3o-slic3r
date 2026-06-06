# PR-7c-7 — Sync-on-send (per-driver metadata emission)

Status: ❌ open.

**Scope.** At send-time, the project's per-(plate, printer)
binding is emitted into the file format each driver consumes.
Bambu reads the binding from `.gcode.3mf` metadata fields;
U1 reads it from G-code header comments.

**Acceptance criteria.**

- **Bambu (`.gcode.3mf` extension)**:
  - Extend PR-3-10's `SlicedPlate` to carry `ams_bindings:
    Vec<AmsBinding>` (already exists; was stub from
    PR-3-10's design).
  - On send (PR-7a-5's path), construct `AmsBinding[]` from
    the plate's current bindings + the printer's
    FilamentState. Each entry maps
    `model_material_index → ams_slot + filament_settings_id`.
  - Populate `Metadata/plate_<N>.json`'s
    `filament_settings_id` array indexed by model material
    (PR-3-10's `SlicedPlateMetadata.filament_used_*`
    already structured by index — same indexing).
  - Verify the printer recognizes the binding: real-print
    smoke (Phase 7c-9) sources from Bambu Studio's known-
    good shape.

- **U1 (G-code header comments)**:
  - Extend PR-3-2's orchestrator output to inject header
    comments after slicing:
    ```
    ; filament_settings_id_0 = generic-pla
    ; filament_settings_id_1 = bambu-petg-hf
    ...
    ```
  - One entry per model material → toolhead slot.
  - The U1 firmware reads these to validate the loaded
    filament matches expected (per Snapmaker's Klipper config).
  - Emit the comment block as part of the pre-slice gcode
    prologue, before the `; HEADER_BLOCK_END` marker.

- **Driver `send()` path**: takes `SendPayload::Gcode3mf` /
  `SendPayload::Gcode` whose bytes already carry the binding
  metadata. The orchestrator (PR-3-2 + PR-6-1 input builder)
  is where the injection happens; drivers just upload.

- **Inject helper** in `core/slice/sync_metadata.rs`:
  - `inject_bambu_bindings(plate: &mut SlicedPlate, bindings: &[MaterialBinding], filament_state: &FilamentState)`
  - `inject_u1_bindings(gcode: &mut Vec<u8>, bindings: &[MaterialBinding], filament_state: &FilamentState)` —
    rewrites the header block.

- Tests:
  - **`bambu_inject_populates_filament_settings_id`** —
    construct a 2-material plate, inject, assert the JSON
    contains both filament_settings_ids in order.
  - **`u1_inject_writes_header_comments_in_order`** —
    construct a 4-material gcode, inject, assert
    `filament_settings_id_0..3` appear in the header block
    in slot order.
  - **`inject_handles_missing_filament_state_gracefully`** —
    no FilamentState entry for a slot → uses the cascade-
    default filament identity, doesn't crash.

**Effort.** ~1.5 days.

**Dependencies.** PR-3-10 (`.gcode.3mf` writer + SlicedPlate),
PR-7c-2 (FilamentState), PR-5-6 (binding shape), PR-7a-5 +
PR-7b-4 (send paths consume the augmented bytes).

**Out of scope.**

- Multi-plate `.gcode.3mf` send (all plates in one file) —
  Phase 7+ already loads single-plate per send.
- Re-binding the AMS on the printer side (driver writes vs
  reads) — out of scope.
