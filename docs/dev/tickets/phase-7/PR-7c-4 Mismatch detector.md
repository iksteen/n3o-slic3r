# PR-7c-4 — Mismatch detector + warn-vs-block

Status: ❌ open.

**Scope.** Compare the active plate's material bindings against
the bound printer's live FilamentState; surface mismatches in
the MaterialBindingPanel + as a pre-slice gate. Three mismatch
classes: family, temperature, color.

**Acceptance criteria.**

- New module `core/filament/mismatch.rs`:
  - `pub struct Mismatch { plate_id, slot, kind: MismatchKind, expected, actual }`
  - `pub enum MismatchKind { Family, TemperatureRange, Color }`
  - `pub fn detect(project: &Project, plate_id: PlateId, filament_state: &FilamentState) -> Vec<Mismatch>`
  - For each material binding (`model_material_index → slot`):
    - Look up the bound filament profile from PR-7c-1.
    - Look up the slot's `effective()` filament from PR-7c-2.
    - If `expected.family != actual.family` → Family
      mismatch.
    - If `|expected.nozzle_temp - actual.nozzle_temp| > 10`
      → TemperatureRange mismatch.
    - If `color_distance(expected, actual) > threshold` (use
      CIE Lab ΔE, threshold ≈ 10) → Color mismatch (always
      informational, never blocking).

- **Pre-slice gate**: extend PR-5-6's pre-slice validator to
  call `detect()` after material bindings check. New error
  variant `SliceStartError::FilamentMismatch(Vec<Mismatch>)`.
  Behavior:
  - Family mismatch → block by default.
  - Temperature mismatch → warn by default.
  - Color mismatch → always informational.
  - Per-mismatch-class warn-vs-block is user-configurable
    via the SettingsPanel under "Slicing → Material
    mismatches" (3 toggles).

- **UI surface in `MaterialBindingPanel`** (PR-5-6):
  - Inline icon + tooltip per row when a mismatch exists.
  - Family: red ⛔, "Expected PLA, slot reports PETG."
  - Temperature: yellow ⚠, "Profile temp 215°C, loaded
    filament typical 240°C."
  - Color: blue ℹ, "Expected red, loaded green."

- **Slice button surface**: when a blocking mismatch exists,
  Slice button is disabled with tooltip linking to the
  binding panel.

- Tests:
  - **`detect_family_mismatch_pla_petg`**.
  - **`detect_temperature_mismatch_outside_band`**.
  - **`detect_no_mismatch_when_family_matches_and_temps_close`**.
  - **`detect_color_mismatch_via_lab_delta`**.
  - **`pre_slice_gate_blocks_on_family_mismatch_by_default`**.
  - **`pre_slice_gate_allows_block_via_user_config`** (set
    family from block → warn, assert slice proceeds).

**Effort.** ~2 days. ΔE color math + the warn-vs-block config
plumbing are the bulk.

**Dependencies.** PR-7c-1 (library), PR-7c-2 (FilamentState),
PR-5-6 (pre-slice gate).

**Out of scope.**

- Per-plate per-slot warn-vs-block overrides (only
  per-mismatch-class for MVP).
- Mismatch surfaces in the preview (the preview is post-slice;
  by then a blocking mismatch would have stopped us).
- Auto-resolve "load this filament" prompts to the printer —
  the printer's own UI handles spool loading.
