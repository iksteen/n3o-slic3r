# PR-5-5 — Plate-level metadata (cycle count + composition order)

Status: ❌ open.

**Scope.** Each plate carries metadata the composition plugin
host (Phase 8) and the PlateCycler plugin (Phase 8) consume:

- **`cycle_count`** (FR-MP-7): how many times the platecycler
  should run this plate. Default 1; integer range 1-999. The
  PlateCycler plugin reads this to expand a 3-plate /
  2-cycle-each project into a 6-print queue.
- **`composition_order`** (FR-MP-7): position in the plate
  composition queue. Default = plate index in
  `Project.plates`. User-reorderable via the same drag
  affordance that reorders plates (cut to Phase 9 if not
  shipped here).

Both fields **survive `.3mf` save/load** (PR-5-8 writes them
into the project's extended namespace) and **don't affect the
slice itself** — slicing produces per-plate G-code; the
PlateCycler plugin uses the metadata to drive the platecycler
hardware.

Owns FR-MP-7.

**Acceptance criteria.**

- `PlateMetadata` already declared in PR-5-1; this ticket
  surfaces it to the UI + the Tauri command surface.

- New commands:
  - `project_set_plate_cycle_count(plate_id, count: u32)` —
    validates `1 <= count <= 999`; returns
    `Result<(), String>`.
  - `project_set_plate_composition_order(plate_id, order: u32)`
    — validates against `Project.plates.len()`; auto-
    reorders other plates' `composition_order` to maintain
    a dense [1..N] sequence (no gaps).

- UI surface:
  - Per-plate **cycle count** badge in `PlateTabs` (PR-5-3):
    small superscript-style chip showing the count when
    `> 1`. Defaults to invisible at count=1 so single-cycle
    projects don't accumulate visual noise.
  - **Click-to-edit** behavior: clicking the badge opens a
    small inline number input (mockup-consistent: same
    pattern as the rename input). Commits on blur / Enter;
    cancels on Escape.
  - **Composition order** is a Phase 9 polish — for MVP
    the order matches the plate position in `Project.plates`
    and changing it requires reordering plates (which is
    itself a Phase 9 cut candidate). Doc this constraint
    in the ticket commit so a future round picks it up.

- Tests:
  - cycle_count round-trips through serde JSON
  - Setting cycle_count outside 1-999 errors
  - composition_order auto-shifts to fill gaps when a
    plate is reordered

**Effort.** ~1 day. Cycle count is a single field + a small
UI badge. Composition order's "real" UX waits for plate
reordering.

**Dependencies.** PR-5-1 (`PlateMetadata` type), PR-5-3
(`PlateTabs` for the badge slot).

**Out of scope.** Plate reordering (composition order's UX
surface). Driving the platecycler hardware (Phase 8).
Reading the metadata from a `.3mf` Bambu Studio authored
(BBS uses a different metadata key; document the mapping in
PR-5-8 if needed).

**Cut candidate.** Per-plate cycle count UI entirely
(saves ~1 day per Execution Plan §7 cut list). Plates
default to cycle=1; user can't change. **Cut LAST** — it
breaks the PlateCycler value prop (the platecycler plugin
in Phase 8 needs varying cycle counts to demonstrate
itself).
