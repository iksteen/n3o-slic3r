# PR-8-8 — read-only filament-state Lua bindings

Status: ❌ open.

**Scope.** Give plugins a read-only view of the active printer's live
filament loadout — per-slot identity (type, color, brand/SKU where
reported), loaded flag, and mismatch state — so plugins can be
material-aware (e.g. "only append the swap macro if all slots loaded",
"warn if a PETG slot is mounted for a PLA job"). Read-only: plugins
never write filament state.

Owns **FR-PL-7** (live filament state, read-only).

**Acceptance criteria.**

- A `filament` table injected into the hook context (available to
  pre-slice, post-slice, pre-send), sourced from the active plate's
  printer binding + the live driver status:
  - `filament.slots()` → array of `{ index, type, color, brand,
    loaded, mismatch }` — one entry per physical slot. Sources:
    - identity/type/color/brand from the driver's per-slot report
      (`DriverExtra::Bambu` AMS slots / `DriverExtra::U1` toolheads)
      and the project's filament binding (`core/filament` +
      Phase 7c's binding model), with manual-override identity
      respected.
    - `mismatch` from Phase 7c's mismatch detector (material-family /
      temperature-band), `nil`/false when no binding to compare.
  - `filament.slot(i)` → one slot or `nil`.
  - `filament.printer()` → `{ model, driver_kind, connected }`.
  - All values are snapshots taken at hook-dispatch time (no live
    cells in Lua); a long-running hook sees a consistent view.

- Strictly read-only: the table and its entries have no setters;
  attempting to assign raises a Lua error (lock the metatable /
  use immutable userdata).

- Graceful when there's no live state: an unbound plate or a
  disconnected printer yields `loaded = false`, identity fields `nil`,
  `printer().connected = false` — never an error. Plugins must be able
  to run offline (slice without a printer connected).

- The binding is assembled host-side and passed into the dispatch
  context; it does not reach back into the driver registry from Lua.

- Tests (with a stubbed filament/driver state, no real printer):
  - `slots()` reflects a stubbed AMS loadout (types/colors/loaded).
  - `mismatch` surfaces a stubbed family mismatch.
  - Disconnected printer → `connected=false`, slots `loaded=false`,
    no error.
  - Write attempt from Lua raises an error.

**Effort.** ~1.5 days.

**Dependencies.** PR-8-3 (host/dispatch context), **Phase 7c**
(filament-state model + mismatch detector), `core/driver` status,
`core/filament`. This is the one binding gated on Phase 7c.

**Out of scope.**

- Writing/binding filament from a plugin (read-only by FR-PL-7).
- Live/streaming updates into a running hook — snapshot at dispatch is
  the contract.
- Exposing raw driver protocol details (AMS humidity, etc.) — only the
  identity/loaded/mismatch surface plugins need.
