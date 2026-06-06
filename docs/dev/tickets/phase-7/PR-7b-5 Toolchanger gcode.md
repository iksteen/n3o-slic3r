# PR-7b-5 — Toolchanger G-code emission validation

Status: ❌ open.

**Scope.** Slice a known multi-material model against the U1
cascade and verify libslic3r emits the expected toolchanger
G-code shape: T-commands at color boundaries, park-and-unpark
macros expanded correctly, no orphan tool references. Mirror of
Spike 2's mixed-nozzle validation but against a real cascade.

**Acceptance criteria.**

- New integration test
  `src-tauri/tests/u1_toolchanger_gcode.rs`:
  1. Load the U1 cascade (PR-7b-1's TOML).
  2. Build a 2-material `SlicingContext` (slot 1 = PLA red,
     slot 2 = PLA blue).
  3. Slice `examples/spike3/fourcolor.3mf` via the orchestrator
     (PR-3-2) — restrict to 2 materials by binding only model
     materials 1 & 2 to slots; ignore 3 & 4.
  4. Parse the resulting G-code (PR-3-6).
  5. Assert:
     - `T0` count + `T1` count both > 0 (both tools used).
     - Tool changes appear at layer-internal color boundaries
       (not just once at start).
     - No `T2` / `T3` references (we only bound 2 slots).
     - Each `T<n>` is preceded by the U1 park macro pattern
       and followed by the unpark macro pattern (string-
       matched against the cascade's `change_filament_gcode`
       template).
     - Filament-aggregate comments (`; filament used [mm] = ...`)
       carry 2 values, not 4.

- **Reference comparison**: capture the same model sliced via
  Snapmaker Orca (manually, once) and check the output into
  `src-tauri/tests/fixtures/u1-reference/2-color-orca.gcode`.
  Assert key structural metrics are within tolerance:
  - Layer count within ±5%.
  - Tool-change count within ±20% (Snapmaker Orca may
    optimize differently — Spike 3 found BBS does ~10× fewer
    toolchanges than raw libslic3r for the same input).
  - Estimated print time within ±15%.

- The test runs as `cargo test --test u1_toolchanger_gcode` —
  same shape as the other phase smokes. Uses the FFI init
  pattern from `phase3_smoke.rs`.

- **Doc**: append a section to `docs/dev/tickets/phase-7.md`'s
  "Implementation notes" describing the reference-comparison
  tolerance + where the Snapmaker Orca fixture came from
  (capture date, slicer version).

**Effort.** ~1.5 days. Most of the time is in Snapmaker Orca
fixture capture + tuning the tolerance gates against real
data.

**Dependencies.** PR-7b-1 (U1 cascade), PR-3-2 (orchestrator),
PR-3-6 (parser), PR-1-6 (filament binding through cascade
adapter).

**Out of scope.**

- Mixed-nozzle validation (different nozzle diameters per
  toolhead). Spike 2 already validated the cascade-side
  expression of this; the slicing-side emission for U1 is
  Phase 7b post-MVP unless the smoke surfaces an issue.
- Send-to-printer validation — PR-7b-9's real-print smoke
  covers it.
- Visual diff of toolchange paths against Snapmaker Orca —
  too noisy; structural metrics (counts + times) are enough.
