# PR-7c-2 — `FilamentState` per-printer model + driver hookup

Status: ✅ done (with mid-flight scope redirect).

**What shipped (2026-05-27):**
- Manual sync button on the slot chip strip pulls live driver
  state into the bound PrinterInstance. Command:
  `printer_instance_sync_from_driver(instance_id, driver_id)`
  reads `driver.status().extra`, projects per-slot updates via
  `core/printer/sync::resolve_updates`, writes through
  `mutate_instance` (one atomic transaction, one
  `printer:instance_changed` emit).
- Bambu: exact `filament_id` (tray_info_idx) match against the
  bundled `FilamentFragmentSummary.filament_id`; miss falls back
  to `generic-<material>`. AMS trays + the external spool
  (vt_tray) both routed.
- U1: keep current identity when its base_type matches reported
  material; mismatch falls back to `generic-<material>`. Color
  always updates.
- Resolver lives at `src-tauri/src/core/printer/sync.rs` with
  the full Bambu/U1 fixture matrix as unit tests.

**Scope redirect from the original spec:**
- No new `FilamentState` model — the existing `PrinterInstance.
  extruders[].slots` carries the state directly. Per the
  redirect "slots are a representation of physical reality, not
  model related so they can just be overwritten during sync,"
  the override/reported split collapsed into one editable slot
  with last-edit-wins semantics.
- No automatic `driver:filament_updated` event flow. Sync is
  manual-button-driven (per "syncing should always be manual by
  the button we just created"). The sync button surfaces error
  state via SyncSlotsLabel's triangle when no driver is
  connected.
- No serde split between override and reported — slot state is
  authoritative and persists to the printer instance TOML.

**Acceptance criteria (original, archived):**

- New module `core/filament/state.rs`:
  - `pub struct FilamentState { per_printer: HashMap<String, PrinterFilamentLoadout> }`
    — keyed by `printer_identity`.
  - `pub struct PrinterFilamentLoadout { slots: Vec<SlotState>, last_updated: SystemTime }`
  - `pub struct SlotState { slot_index: u8, reported: Option<ReportedFilament>, override: Option<UserOverride> }`
  - `pub struct ReportedFilament { material_type: String, color: String, sub_brand: Option<String>, source: ReportedSource }`
    — `ReportedSource` is `Bambu` or `U1`.
  - `pub struct UserOverride { profile_id: String, set_at: SystemTime }`
    — profile_id is a `FilamentProfile` identity from PR-7c-1.

**Scope.** The cross-cutting state model that ties printer-
reported live filament state to project-level material bindings
and to the filament library (PR-7c-1). Lives at project scope so
the same project resolves correctly when sliced for either
printer.

**Acceptance criteria.**

- New module `core/filament/state.rs`:
  - `pub struct FilamentState { per_printer: HashMap<String, PrinterFilamentLoadout> }`
    — keyed by `printer_identity`.
  - `pub struct PrinterFilamentLoadout { slots: Vec<SlotState>, last_updated: SystemTime }`
  - `pub struct SlotState { slot_index: u8, reported: Option<ReportedFilament>, override: Option<UserOverride> }`
  - `pub struct ReportedFilament { material_type: String, color: String, sub_brand: Option<String>, source: ReportedSource }`
    — `ReportedSource` is `Bambu` or `U1`.
  - `pub struct UserOverride { profile_id: String, set_at: SystemTime }`
    — profile_id is a `FilamentProfile` identity from PR-7c-1.

- **Resolution helper** (`SlotState::effective() -> Option<&FilamentProfile>`):
  - If `override` is Some → look up that profile from
    `FilamentLibrary`.
  - Else if `reported` is Some → match by `material_type`
    (and color where useful) to a profile family. Use the
    family-match heuristic: `material_type = "PLA"` →
    `generic-pla`; `sub_brand = "Bambu PLA Basic"` →
    `bambu-pla-basic`.
  - Else → None (slot empty / unknown).

- **Driver hookup**:
  - The driver's status worker (PR-7a-3 / PR-7b-3) emits a
    Tauri event `driver:filament_updated` with payload
    `{ driver_id, slots: Vec<ReportedFilament> }` whenever
    the AMS / per-toolhead state changes.
  - A new backend listener consumes the event and updates
    `FilamentState.per_printer[printer_identity].slots[i].reported`.
  - `last_updated` updates each time.

- **`FilamentState` is project-scoped**: lives on `Project`,
  serialized as part of PR-5-8's `.3mf` save. The reported
  fields are NOT serialized (live state, not project data);
  only `override` survives save/load.

- **Tauri commands** (`core/filament/state_commands.rs`):
  - `filament_state_get(printer_identity) -> Option<PrinterFilamentLoadout>`.
  - `filament_state_set_override(printer_identity, slot, profile_id)`.
  - `filament_state_clear_override(printer_identity, slot)`.
  - `filament_state_refresh(printer_identity)` — trigger a
    driver re-query.

- Tests:
  - **`effective_prefers_override_over_reported`**.
  - **`effective_resolves_reported_by_family_match`**.
  - **`effective_returns_none_for_empty_slot`**.
  - **`override_survives_serde_roundtrip`** (project save / load).
  - **`reported_does_not_survive_serde_roundtrip`** (live
    state filtered out).
  - **`driver_event_updates_reported_field`** — wire test
    with a synthetic `driver:filament_updated` payload.

**Effort.** ~2 days. Most of it is the project-state
integration + the family-match heuristic.

**Dependencies.** PR-7a-4 (Bambu AMS shape), PR-7b-3 (U1
per-toolhead shape), PR-7c-1 (FilamentLibrary), PR-5-1
(Project shape).

**Out of scope.**

- Color-match heuristic for auto-binding (PR-7c-5 owns it).
- Mismatch detection (PR-7c-4).
- UI surfaces (PR-7c-3).
