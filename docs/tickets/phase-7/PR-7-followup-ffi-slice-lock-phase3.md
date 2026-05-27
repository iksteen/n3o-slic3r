# PR-7 follow-up — FFI slice-callback lock in phase3_smoke (and slic3r-ffi/api)

Status: ❌ open.

**Scope.** libslic3r's progress callback (`set_slice_progress`) is a
single process-global slot. When two tests in the same binary run
slice jobs in parallel, the second `set_slice_progress` call
overwrites the first — and the first job's progress events,
including `PlateFinished`, silently vanish. The test then panics on
the missing event with no hint of the underlying race.

`phase_s_smoke.rs:49` already pins the pattern: a
`static FFI_SLICE_LOCK: Mutex<()>` acquired at the top of every
slice helper so concurrent test threads serialize on the callback
slot. `phase3_smoke.rs` and `crates/slic3r-ffi/tests/api.rs` predate
that pattern and have the latent race.

**Trigger.** CI run `26482183987` on commit `9d91ccf` failed with
`phase3_smoke_slice_parse_roundtrip_bundle` panicking at
`.expect("PlateFinished event with summary")`. Reproduces only under
concurrent test scheduling (passed 3/3 in isolation locally on the
same commit). Same flake observed mid-session on `api.rs` (gridded
with `phase_s_smoke`'s lock-protected leg, the unprotected api.rs
test lost its events).

**Acceptance criteria.**

- **`phase3_smoke.rs`**: add `static FFI_SLICE_LOCK: Mutex<()>` and
  acquire it at the top of `slice_cube_to_gcode`. Mirrors
  `phase_s_smoke.rs:49` verbatim.
- **`crates/slic3r-ffi/tests/api.rs`**: same treatment — every test
  that calls `set_slice_progress` or `slice` takes the lock first.
- **Both files**: convert the `find_map(...).expect("PlateFinished")`
  pattern to first scan for `SliceEvent::JobFailed` and panic with
  its `error` field. Pre-existing failures stay legible; the lock
  fix should make this dead code in practice but the diagnostic
  affordance pays for itself the first time the flake mutates.
- **Optional**: hoist the lock into the `slic3r-ffi` crate as a
  `pub static` so future test files reuse one canonical mutex
  instead of each binary defining its own. Defer until a third
  binary needs it — premature factoring otherwise.

**Out of scope.** Cross-binary serialization (cargo runs separate
test binaries in parallel processes; you'd need a file lock or
`--test-threads=1`). The per-binary fix is enough — flakes
historically only fire within one binary's parallel test scheduler.

**Effort.** ~30 minutes. Mechanical edit + a re-run pass to confirm
the previously-flaky test is stable.

**Dependencies.** None.
