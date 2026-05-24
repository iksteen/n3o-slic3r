# PR-7c-2 — `FilamentState` per-printer model + driver hookup

Status: ❌ open.

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
