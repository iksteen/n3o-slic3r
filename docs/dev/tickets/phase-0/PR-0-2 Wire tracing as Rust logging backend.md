# PR-0-2 — Wire `tracing` as the Rust logging backend

Status: ✅ done (commit `bed94d0`).

**Scope.** Add `tracing` + `tracing-subscriber` to the Rust crates,
initialize a subscriber early in src-tauri's `run()`, and replace ad
hoc `eprintln!` / `println!` debug output with `tracing::info!` etc.
Filter via `RUST_LOG`. Logs go to stderr in a human-readable format
by default; structured JSON output gated by an env var for future CI
ingestion.

**Acceptance criteria.**

- `tracing = "0.1"` and `tracing-subscriber = { version = "0.3",
  features = ["env-filter", "fmt", "json"] }` in
  `src-tauri/Cargo.toml`.
- src-tauri's `run()` calls a subscriber init function early
  (before `tauri::Builder::default()`) that honors `RUST_LOG`
  (defaulting to `info`) and emits to stderr.
- At least one `tracing::info!` event in each of the three Tauri
  commands (`slicer_info`, `slicer_options`, `slicer_slice`) with
  span context for the command name.
- A `LOG_FORMAT=json` env var switches the output format from
  pretty-text to JSON Lines.
- `RUST_LOG=debug npm run tauri dev` produces filtered logs with
  timestamps, levels, target paths, and the span chain.
- Manual smoke: clicking "Search" with a filter logs the
  `slicer_options` invocation; clicking "Slice" logs the
  `slicer_slice` invocation with model path and out path.

**Effort.** Half a day.

**Dependencies.** None. Independent of PR-0-1, PR-0-3.

**Out of scope.** Replacing libslic3r's own `boost::log` output (the
"Logging sink redirect" item in PRD §8.3) — that's a separate FFI
extension. Today libslic3r still goes to stderr directly via boost.

**Implementation note (post-delivery).** The ticket originally also
asked for `tracing` to be added to `crates/slic3r-ffi/Cargo.toml`
for future library-side events. That dep was added then removed in
the same commit — the FFI crate doesn't emit any events today, and
unused deps are noise. Add it when the first event is needed.
