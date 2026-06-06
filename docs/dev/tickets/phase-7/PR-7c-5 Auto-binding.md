# PR-7c-5 — Auto-binding heuristic (family match on first assignment)

Status: ❌ open.

**Scope.** On first plate-to-printer binding, attempt to bind
model materials to physical slots based on filament-family
match. User confirms or adjusts. PR-5-6's MaterialBindingPanel
gets an "Auto-bind" button driven by this heuristic.

**Acceptance criteria.**

- New module `core/filament/auto_bind.rs`:
  - `pub fn auto_bind(model_materials: &[ModelMaterial],
       loadout: &PrinterFilamentLoadout) -> Vec<MaterialBinding>`
  - Algorithm:
    1. For each `model_materials[i]`:
       - Find first unbound slot in `loadout` whose
         `effective().family == model_materials[i].family`.
       - If found, bind. Mark slot as used.
       - If not found, bind to slot 1 with a
         `BindingWarning::FamilyMismatch` annotation.
    2. Tie-break by color proximity (ΔE) when multiple
       same-family slots are available.

- **`MaterialBindingPanel` "Auto-bind" button** (extends
  PR-5-6):
  - Visible when:
    - Plate has unresolved bindings.
    - Printer is connected + FilamentState reports ≥1 loaded
      slot.
  - On click: calls `filament_auto_bind(plate_id)` Tauri
    command which runs `auto_bind()` and applies the
    resulting bindings. Surfaces the warnings inline.

- **Re-binding** (when user changes printer mid-project):
  - Auto-bind doesn't re-fire automatically — it would
    surprise the user. Instead, the
    PrinterCredentialsDialog → PrinterPicker offers
    "Auto-bind materials for new printer?" checkbox on the
    confirmation step.

- Tests:
  - **`auto_bind_pla_to_pla_slot`** — 1 model material (PLA)
    + loadout (slot 1 PLA, slot 2 PETG) → binding goes to
    slot 1.
  - **`auto_bind_tie_breaks_by_color`** — 1 model material
    red PLA + loadout (slot 1 PLA black, slot 2 PLA red) →
    binding goes to slot 2.
  - **`auto_bind_falls_back_to_slot_1_with_warning`** — model
    PETG + loadout (all PLA) → bound to slot 1 with
    FamilyMismatch warning.
  - **`auto_bind_skips_already_bound_slots`** — 2 model
    materials of same family + 4 same-family slots → each
    gets a distinct slot.

**Effort.** ~1.5 days.

**Dependencies.** PR-7c-1 (library), PR-7c-2 (FilamentState),
PR-5-6 (existing binding panel).

**Out of scope.**

- ML-based color matching (use straightforward ΔE).
- Persistent learning from user corrections — Phase 9+.
- Multi-pass optimization (Hungarian assignment) — greedy
  first-match is sufficient for ≤ 4 slots.
