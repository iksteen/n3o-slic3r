# PR-4-6 — Slot-adaptive layout (per-slot tab strip)

Status: ❌ open.

**Scope.** Printers with `slot_count >= 2` render the per-slot
configuration surface as a tab strip (one tab per slot) with a
"Sync edits across slots" toggle that defaults ON. Single-slot
printers (A1 mini single-extruder, FDM hobbyist defaults) skip the
tab strip entirely and render a flat panel.

Owns FR-UI-8 (slot-adaptive layout).

**Acceptance criteria.**

- New `src/settings/slots/SlotTabStrip.tsx`:
  - Hidden when `printer.slot_count == 1`.
  - When `slot_count >= 2`, renders one tab per slot with the
    slot's filament-color swatch + index ("1", "2", "3", "4" for
    A1 mini's 4 AMS slots; for the U1 use the toolhead's
    nozzle-diameter label). Active slot highlighted.
  - Above the tabs, a "Sync edits across slots" toggle
    (default ON). When ON, edits to vector options
    (`is_vector` per PR-4-1) on the active tab broadcast to the
    matching index across all tabs.
  - Sync state persists in localStorage keyed by
    `n3o.settings.sync_slots`.

- Vector-option rendering:
  - The settings panel's row for a vector option (e.g.
    `filament_type`, `nozzle_temperature`) displays only the
    active slot's value. The vector index is the active slot
    index minus one (1-based slot → 0-based vector index).
  - Edits land at the active slot's index. With sync ON, edits
    apply to **every** index simultaneously.
  - Vector length is the active printer's `slot_count` (slot
    bindings) or `len(toolheads)` (per-extruder options like
    `nozzle_diameter`) — the panel resolves which based on the
    `OptionSummary` metadata. Dimensions don't change at runtime
    (slot count is printer-fixed); if the cascade resolves to
    fewer entries than slots, the panel pads with the cascade's
    last value (matches libslic3r's `get_at` wrap-extend
    semantics).

- Smoke check (exit criterion verifier):
  - Mount the panel with the A1 mini profile → tab strip absent.
  - Mount with the U1 profile (4 toolheads) → 4-tab strip
    visible; sync toggle defaults ON.
  - Sync ON: edit `nozzle_temperature` on tab 2; tabs 1/3/4 all
    show the same new value.
  - Sync OFF: edit `nozzle_temperature` on tab 2; only tab 2
    changes.

- vitest:
  - SlotTabStrip elides when `slot_count == 1`.
  - Sync ON broadcasts vector edits to all indices; sync OFF
    constrains to the active index only.
  - Vector index math: slot 1 → index 0, slot 4 → index 3.

**Effort.** ~2 days. The tab strip + sync toggle is a day; the
vector-option index plumbing into the panel + form components is
another day (touches PR-4-2's MultiSelectInput contract).

**Dependencies.** PR-4-1 (vector flag), PR-4-2 (form components),
PR-4-4 (panel scaffold), PR-1-7 (printer profile w/ slot_count +
toolheads).

**Out of scope.** Adding/removing slots at runtime (slot count is
printer-fixed in MVP). Per-slot filament-spool-brand UI — that's a
Phase 7c (filament sync) responsibility.

**Cut candidate.** The "Sync edits across slots" toggle (per
Execution Plan §6 cut list — saves 2 days). Users would edit each
tab independently. Hurts UX for the common "configure all toolheads
identically" case; cut last. If cut, the tab strip ships in
"independent edit" mode by default and users get a Phase 9 polish
ticket for the sync affordance.
