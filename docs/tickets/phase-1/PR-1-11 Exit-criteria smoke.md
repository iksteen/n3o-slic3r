# PR-1-11 — Phase 1 exit-criteria smoke

Status: ⚠️ partial. `cargo run -p n3o-slic3r --release --example phase1_smoke` drives PR-1-1 through PR-1-8 end-to-end against the reference profiles; `docs/phase-1-smoke.md` documents the procedure. The validation-errors companion example, the bench wiring (PR-1-10), and the CI hook are deferred — the unit + integration tests already gate cascade behavior in CI, so the user-facing smoke binary is the primary remaining gap.

**Scope.** End-to-end smoke procedure that exercises Phase 1's
exit criteria as a single repeatable test. Mirrors PR-0-5's
Phase 0 smoke: documented procedure + CLI driver that runs it
locally and a CI hook so regressions are caught early.

**Acceptance criteria.**

- `docs/phase-1-smoke.md` documents the procedure step-by-step:
  1. `cargo test --workspace --release` — all PR-1-* tests pass,
     including resolver / adapter / trace / override unit tests.
  2. `cargo run --example phase1_smoke` — drives the full pipeline:
     - Loads `profiles/cascades/bambu-a1-mini-default.toml`.
     - Loads `profiles/printers/bambu-a1-mini.json` +
       `profiles/plates/textured-pei.json` +
       `profiles/filaments/generic-pla.json`.
     - Builds `Context` with slot 0 = PLA.
     - Runs `resolve_with_overrides`, asserts `bed_temp = 65` and
       `nozzle_temperature = 220` per the cascade.
     - Calls `cascade_trace` for `bed_temp`, prints the
       structured trace (winner + 2 losers).
     - Applies override: `project.bed_temp = 50`. Re-resolves,
       asserts new value + cascade_fallback.
     - Calls `adapt`, slices `OrcaCube_v2.3mf`, writes
       `/tmp/phase1.gcode`, asserts non-empty.
  3. `cargo run --example phase1_smoke -- --u1` — same procedure
     for U1 + textured PEI + PLA slot 0 + PETG slot 1; asserts
     per-slot resolution (slot 1's nozzle_temperature ≠ slot 0's).
  4. Load-time validation:
     `cargo run --example phase1_validation_errors` exercises
     each of the three error classes (misspelled predicate,
     unknown set key, scope violation) and confirms each fails
     load with the expected message.
  5. Benchmark check: `cargo bench --bench cascade` completes
     with all asserts (budget) green.

- CI hook in `.github/workflows/build.yml`: add a step
  `cargo run --release --example phase1_smoke` after the
  workspace tests pass.

- The smoke procedure runs cleanly from a clean checkout. Any
  divergence from documented expected output is recorded as a
  bug or a documentation update.

**Effort.** ~1 day.

**Dependencies.** All other Phase 1 tickets complete.

**Out of scope.** GUI-level smoke (Phase 4 will add that).
Printer hardware connectivity smoke (Phase 7). Performance
regression alerts beyond the bench `assert!` (Phase 9 release
gating could add bench-results-to-PR-comment).
