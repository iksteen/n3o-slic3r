# PR-7c-6 — Per-(plate, printer) binding persistence

Status: ❌ open.

**Scope.** Bindings are stored per-(plate, printer) so reassigning
a plate from A1 mini to U1 surfaces the U1's stored binding or
prompts for one. Today's `Plate.material_bindings` is single-
printer-implicit; this ticket adds the dimension.

**Acceptance criteria.**

- **Refactor `Plate.material_bindings`**:
  - From: `Vec<MaterialBinding>` (implicit current printer).
  - To: `HashMap<String, Vec<MaterialBinding>>` — keyed by
    `printer_identity`.
  - All read paths (cascade context build, pre-slice gate,
    binding panel) look up by the plate's CURRENT printer.

- **Plate-printer reassignment behavior**:
  - When user changes a plate's printer in the picker:
    1. Save the current bindings under the OLD printer's key.
    2. Look up bindings for the NEW printer's key.
    3. If found → apply directly.
    4. If not found → either run auto-bind (if the user opted
       in via the picker's checkbox) or surface the
       MaterialBindingPanel with an empty binding state.

- **Serde**: `Plate.material_bindings` round-trips as a JSON
  object (printer-identity → bindings array). Extend PR-5-8's
  `.3mf` save/load to handle the new shape. Old-format `.3mf`
  files (pre-Phase-7c) migrate: existing bindings get
  promoted to a key matching whatever printer the plate was
  bound to at save time.

- **Migration path**: `read_project` detects old-format
  bindings (an array instead of an object) and converts via a
  `migrate_bindings_v1_to_v2` helper. Adds a tracing warning
  noting the migration happened.

- Tests:
  - **`bindings_keyed_by_printer_identity`** — set bindings
    on A1 mini, switch plate to U1, assert empty bindings.
    Switch back to A1 mini, assert original bindings restored.
  - **`save_load_roundtrips_per_printer_bindings`** —
    multi-printer state survives a `.3mf` round-trip.
  - **`migrate_v1_v2_promotes_existing_bindings_under_current_printer_key`** —
    legacy `.3mf` loads with bindings preserved.
  - **`auto_bind_checkbox_runs_on_printer_change`** — wire
    test for the picker confirmation.

- **Project autosave** (PR-5-10) verified to capture the new
  shape — no extra work expected, but spot-check.

**Effort.** ~2 days. The serde migration + extending the
plate-switch flow are the bulk.

**Dependencies.** PR-5-6 (MaterialBindingPanel), PR-5-8
(project save/load), PR-5-10 (autosave).

**Out of scope.**

- Cross-printer binding sync ("link these two plates' bindings")
  — too clever for MVP.
- Per-printer override hierarchies for cascade overrides
  (also project-tier, also stored per-printer?) — out of
  scope; overrides stay per-plate-only.
