# PR-7b-1 — Snapmaker U1 cascade TOML

Status: ❌ open.

**Scope.** Author `profiles/cascades/snapmaker-u1-default.toml`
— the cascade-shaped settings bundle for the U1, mirroring
`bambu-a1-mini-default.toml`'s shape and authoring style. Plus
the per-toolhead default surfaces (4× 0.4mm steel as shipped).
Plus the start/end gcode + tool-change macros sourced from
Snapmaker Orca's published profile.

**Acceptance criteria.**

- New file `profiles/cascades/snapmaker-u1-default.toml`:
  - `[meta]`: `name = "Snapmaker U1 default"`, `printer =
    "snapmaker-u1"`.
  - Print parameters cascade (mirror A1 mini's structure):
    `layer_height`, `first_layer_height`, `wall_loops`,
    `sparse_infill_density`, `sparse_infill_pattern`,
    `default_acceleration`, etc.
  - Material parameters: PLA / PETG / ABS variants with
    appropriate temps for each.
  - Build-plate parameters: `bed_temperature` per plate type
    (Magnetic / PEI / Glass — pick from the existing U1
    printer profile's `supported_build_plates`).
  - Per-toolhead overrides via `when.slot = N` predicates:
    default all 4 slots to the same 0.4mm steel config; the
    user adjusts via the cascade UI if a slot has a different
    nozzle.

- **Start G-code template** sourced from Snapmaker Orca's
  published U1 profile (commit a known SHA in a code comment).
  Includes:
  - Bed heat / nozzle preheat (per-active-toolhead).
  - Home + probe.
  - Initial purge.
  - The U1-specific carriage parking macro (only the active
    toolhead unparks).

- **End G-code template**:
  - Park all toolheads.
  - Bed cool / nozzle cool.
  - Home X.
  - Disable motors.

- **Tool-change macro** (`change_filament_gcode` equivalent
  for libslic3r emission):
  - Park current toolhead.
  - Unpark next toolhead.
  - Purge.
  - Return to the print position.

- **Plate list** (already in the printer profile, here just
  used as the `when.build_plate.identity` selector domain):
  Magnetic Steel, Smooth PEI, Textured PEI, Glass. Verify the
  list against the actual U1 plate accessories Snapmaker sells.

- Tests:
  - `cargo test -p n3o-slic3r --lib core::cascade -- snapmaker_u1`
    — loads the cascade, validates against the schema (PR-1-1),
    resolves a sample context (U1 + PEI plate + PLA slot 1),
    asserts every required key resolves to a non-default value.
  - **`per_toolhead_nozzle_override_resolves`** — synthesize a
    context with `slot=2` having an overridden nozzle diameter
    (0.6mm), assert `nozzle_diameter` resolves to 0.6 for slot
    2 and stays 0.4 for slots 1, 3, 4. (Validates PR-1-2's
    `when.slot` predicate end-to-end against the U1 cascade.)

**Effort.** ~1.5 days. Authoring the cascade is the bulk; the
start/end gcode is a transcribe-from-Snapmaker-Orca step.

**Dependencies.** PR-1-2 (cascade format), PR-1-8 (A1 mini
cascade as reference shape), existing `profiles/printers/snapmaker-u1.toml`.

**Out of scope.**

- Per-toolhead non-default nozzle sizes — the cascade permits
  it via `when.slot = N` predicates but defaults to 4× 0.4mm
  steel. PR-7b-7 surfaces the per-toolhead override UX.
- Multi-material cascade composition (PETG on slot 1 + ABS on
  slot 2 simultaneously) — supported via existing filament
  binding mechanism, no Phase 7 cascade work needed.
- Toolchanger sequence optimization (minimizing tool changes
  via clever ordering) — libslic3r handles this; we just emit
  the macro.
