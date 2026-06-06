# PR-0-5 — Phase 0 exit-criteria smoke

Status: ✅ done. Procedure documented at `docs/dev/phase-0-smoke.md`.

**Scope.** A concrete script and checklist that exercises Phase 0's
exit criteria end-to-end. Document it so the same smoke runs after
the libslic3r submodule bump, the cargo toolchain bump, etc.

**Acceptance criteria.**

- `docs/dev/phase-0-smoke.md` documents the smoke procedure:
  1. `git submodule update --init --recursive`
  2. `./scripts/build.sh deps` (skip if already built)
  3. `cargo test --workspace --release` — 16/16 tests pass.
  4. `cargo run -p slic3r-ffi --release --example introspect` —
     prints `OrcaSlicer libslic3r_ffi v0` and `total options: 737`
     (or N if upstream changed).
  5. `cargo run -p slic3r-ffi --release --example slice -- <test
     STL> /tmp/out.gcode` — example *runs* and surfaces libslic3r's
     `Print::validate()` rejection of the FullPrintConfig defaults
     ("Relative extruder addressing requires resetting the extruder
     position at each layer ... Add 'G92 E0' to layer_gcode"). A
     successful slice with non-empty gcode is **Phase 0.5 / Spike 1**
     work; Phase 0 only verifies the FFI link reaches `Print::apply`.
  6. `npm install && npm run tauri dev` — app window launches,
     header shows the libslic3r version + option count. The Slice
     form surfaces the same validate gap as step 5; that is the
     expected Phase 0 result.
- The smoke procedure runs cleanly from a clean checkout. Any
  divergence from the documented expected output is recorded as a
  bug or a documentation update.

**Effort.** Half a day, including running the procedure once to
confirm.

**Dependencies.** PR-0-1, PR-0-2, PR-0-3 complete. PR-0-4 is
independent (CI runs the same smoke).

**Out of scope.** Anything that touches printer hardware
(connectivity is Phase 7). Anything in the renderer beyond
"launches and displays version." Multi-printer workflows (Phase 5).

**Discovery during this ticket.** The slice example fails
`Print::validate()` against FullPrintConfig defaults on every
fixture tried (STL, OrcaCube_v2.3mf, test_3mf/Büchse.3mf). After
discussion the call was to *not* patch the example — instead the
Phase 0 smoke documents this as the expected gap and Spike 1 picks
up the real round-trip from a converted OrcaSlicer device profile.
That constraint is now encoded in `docs/dev/Execution_Plan.md` Spike 1,
`docs/dev/profiles.md` "What stays libslic3r-shaped", and the Phase 0.5
PR-0.5-1 ticket.
