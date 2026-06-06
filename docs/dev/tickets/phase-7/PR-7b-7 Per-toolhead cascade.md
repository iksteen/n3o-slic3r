# PR-7b-7 — Per-toolhead independent nozzle/hotend cascade wiring

Status: ❌ open.

**Scope.** Surface the U1's per-toolhead configurability through
the settings panel. PrinterProfile already has `Vec<Toolhead>`
with per-slot fields (PR-1-7); `when.slot = N` predicates exist
(PR-1-2). This ticket wires the per-slot UI into the existing
SettingsPanel's slot-tab strip (PR-4-6) so a user can override
slot 2 to have a 0.6mm nozzle while slots 1/3/4 stay 0.4mm.

**Acceptance criteria.**

- **Cascade resolution** (already supported by PR-1-2 +
  PR-1-3 — verify only):
  - A `nozzle_diameter` cascade rule with `when.slot = 2` and
    `value = 0.6` resolves to 0.6 for slot 2 contexts and
    stays at the default for slot 1/3/4 contexts.
  - Add a test in `core/cascade/` that covers this against
    the U1 cascade.

- **Settings panel surface**:
  - The slot-tab strip (PR-4-6) already exists; verify it
    shows 4 tabs for U1 contexts.
  - For each slot, enable per-slot override authoring on
    `nozzle_diameter`, `hotend_type`, `max_nozzle_temperature`,
    `slot_color`. The override-write path (PR-4-9's Reset
    button + override-tier tinting) just works against the
    existing slot-scoped predicate.

- **Per-toolhead state surface** (this is the user-facing
  diff vs A1 mini):
  - The slot-tab strip's label includes the **mounted-toolhead
    indicator**: the currently-mounted slot's tab has a ring
    (mirror the AMS active-slot ring from PR-7a-4).
  - When the driver reports `U1Extra.mounted_toolhead`
    changing (PR-7b-3's `driver:toolhead_changed` event),
    the indicator follows.

- **`SyncEdit` opt-out** (per PR-4-6's mechanism): for U1
  contexts, per-toolhead settings (nozzle_diameter,
  hotend_type, max_temp) default to non-synced so editing
  slot 1 doesn't broadcast to slots 2-4 (mirror the U1's
  physical independence).

- Tests:
  - **`cascade_resolves_per_slot_nozzle_diameter`** — unit
    test against the U1 cascade with `when.slot = 2` override.
  - **`settings_panel_slot_strip_shows_4_tabs_for_u1`** —
    frontend vitest.
  - **`mounted_toolhead_indicator_updates_on_event`** —
    frontend vitest with a stubbed `driver:toolhead_changed`
    event.

**Effort.** ~1.5 days. Most of it is the indicator wiring +
the sync-opt-out default — the cascade-side mechanics already
exist from Phase 1.

**Dependencies.** PR-1-2 (when.slot predicate), PR-4-6
(SlotTabStrip + SyncEdit), PR-7b-1 (U1 cascade), PR-7b-3
(toolhead-changed event).

**Out of scope.**

- Surfacing per-toolhead nozzle wear (Phase 9+ if Snapmaker
  starts reporting it).
- Drag-to-reorder toolhead slots in the UI — the physical
  layout is fixed by the printer.
- Migration UI for users who had A1-mini-bound plates and
  reassign to U1 mid-project — handled by PR-7c-6's
  per-(plate, printer) binding persistence.
