# PR-3-1 — FFI extensions: slice progress callback + log sink redirect

Status: ❌ open.

**Scope.** Add the two remaining FFI extensions called out in PRD
§8.3 that Phase 3 depends on. C++/Rust work in `crates/slic3r-ffi/`.

- **Slice progress callback.** Today `slic3r_ffi::slice` is one
  blocking call with no progress signal. Phase 3's slice
  orchestration (PR-3-2) needs to stream progress events to the UI
  on a worker thread. Wire a `slic3r_set_slice_progress_cb(fn,
  user_data)` (or per-call `slice_with_progress(...)`) that fires
  from libslic3r's existing progress reporting.
- **Logging sink redirect.** Today libslic3r's `boost::log` defaults
  go to stderr. Phase 3+ wants these events routed through Rust's
  `tracing` subscriber so they end up in the same JSON log stream
  the rest of the app emits. Wire a `slic3r_set_log_sink(fn,
  user_data)` callback.

Both extensions are first-party (PRD §8.3 — FFI is in-house), so
risk is execution time, not external dependency.

**Acceptance criteria.**

- C-level API:
  - `int slic3r_set_slice_progress_cb(slic3r_progress_fn_t cb,
    void* user_data)` — registers a callback invoked from inside
    `slic3r_slice`. Callback signature carries `(percent: float,
    stage: const char*, user_data: void*)`. Stages match
    libslic3r's existing `PrintBase::SlicingStatus` strings.
  - `int slic3r_set_log_sink(slic3r_log_fn_t cb, void* user_data)`
    — registers a callback invoked for every `boost::log` record.
    Callback carries `(severity, message, user_data)` with severity
    mapped to a small enum: trace / debug / info / warn / error.

- Rust binding under `slic3r_ffi`:
  - `pub fn set_slice_progress(cb: impl FnMut(f32, &str) + Send +
    'static)` — owns the trampoline + `Box::into_raw` lifetime.
  - `pub fn set_log_sink(cb: impl FnMut(LogLevel, &str) + Send +
    'static)`. `LogLevel` enum mirrors libslic3r's severity ladder.

- Survey the FFI for the **G-code-to-memory-buffer** path (PRD §8.3
  flags it as "if not already supported"). If absent, add it here
  as a third extension; if present, leave a doc-comment pointer in
  this ticket's PR description so PR-3-2 knows where to plug in.

- Thread safety: both callbacks fire from libslic3r's slice thread.
  The Rust trampoline must hold the callback behind a `Mutex` (or
  channel) so the closure can be `FnMut + Send` without `Sync`.
  Document this in the rustdoc — Phase 8's plugin system will need
  the same pattern.

- Tests:
  - C-level: a probe binary registers a callback, runs a 1-cube
    slice, asserts the callback fires at least once per stage.
  - Rust: integration test under `crates/slic3r-ffi/tests/` slices
    `examples/spike1`'s cube, collects progress samples, asserts
    monotonic non-decreasing percent values and at least one
    `"Generating support material"` stage tick (or whichever stage
    is reliably emitted for the fixture).

- Docs: extend `docs/libslic3r-workarounds.md` with a new section
  describing where in libslic3r the progress + log hooks were
  added, what the dispatch policy is (every record vs. throttled),
  and how to extend the severity mapping when upstream changes.

**Effort.** ~2 days. Most of the time is the C++ side — wiring a
`std::function` registry into libslic3r's static-singleton style
isn't free. The Rust trampolines are mechanical.

**Dependencies.** Phase 3 (this is the first Phase 3 ticket — runs
in parallel with PR-3-5/6 if scheduling allows since they're parser-
only). Touches `crates/slic3r-ffi/ffi/` and `crates/slic3r-ffi/src/`.

**Out of scope.** Throttling progress events to a maximum rate (the
slice command on the Rust side can debounce). Streaming structured
events beyond "percent + stage" — the Slicer's existing reporting
shape is what we get; richer telemetry is post-MVP.

**Risk: this is the only Phase 3 ticket that requires C++ work.**
If the libslic3r-side wiring proves nontrivial (locking, callback
storage, ABI), the implementer should surface that early — the rest
of Phase 3 can advance with a temporary "no progress events" stub
in PR-3-2 while PR-3-1 closes out independently.
