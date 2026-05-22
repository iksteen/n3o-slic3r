# Phase 0 — tickets

Phase 0 (Foundation, ~2 person-weeks) is partially complete. This
document tracks the remaining work as concrete tickets and confirms
what's already done.

Source: `docs/Execution_Plan.md` §2. The phase's stated exit criteria:

> App launches on the project lead's primary dev machine. Frontend
> shows libslic3r version. CI green on Linux.

## Status by deliverable

| Deliverable | Status | Notes |
|-------------|--------|-------|
| Tauri 2.x project scaffolded, React + TS frontend wired | ✅ done | commit `5f4a34d` |
| Tailwind frontend wired | ❌ open | **P0-1** |
| orca-slicer-ffi linked into the Tauri core | ✅ done | vendored at `crates/slic3r-ffi`; src-tauri depends via path |
| Tauri command exposes `slic3r_version()` to the frontend; UI displays it | ✅ done | `slicer_info()` returns version + option count; rendered in `App.tsx` |
| Linux CI building | ❌ open | **P0-4** |
| Logging infrastructure (`tracing` crate) wired into Rust core | ❌ open | **P0-2** |
| Repo structure matches PRD §8.2 module boundaries | ⚠️ partial | **P0-3** (core/ subtree doesn't exist yet) |

Three open tickets cover the gaps; **P0-5** runs the phase-0 exit
criteria as a single smoke procedure.

---

## P0-1 — Add Tailwind CSS to the frontend

**Scope.** Install Tailwind, configure it for Vite + React, replace
the inline-style approach in `App.tsx` with a small smoke-test of
utility classes to prove the pipeline. Don't restyle the whole UI
yet — that's Phase 4 (Settings UI) work.

**Acceptance criteria.**

- `tailwindcss`, `postcss`, `autoprefixer` in `package.json`'s
  `devDependencies`.
- `tailwind.config.js` exists with `content: ["./index.html",
  "./src/**/*.{ts,tsx}"]`.
- `postcss.config.js` exists referencing `tailwindcss` and
  `autoprefixer`.
- `src/index.css` (or equivalent) contains the three
  `@tailwind base/components/utilities;` directives.
- `App.tsx` uses at least one Tailwind class (e.g. wrap the header
  in `<h1 className="text-2xl font-semibold mb-4">`) and renders
  styled in `npm run tauri dev` — verifiable by opening the app and
  seeing the styled element.
- The existing inline styles in `App.tsx` remain functional during
  the transition (don't break the running UI).

**Effort.** Half a day.

**Dependencies.** None.

**Out of scope.** Restyling the introspection table, the slice form,
or any other surface beyond the smoke header. Theme customization,
dark mode, design tokens — all Phase 4.

---

## P0-2 — Wire `tracing` as the Rust logging backend

**Scope.** Add `tracing` + `tracing-subscriber` to the Rust crates,
initialize a subscriber early in src-tauri's `run()`, and replace ad
hoc `eprintln!` / `println!` debug output with `tracing::info!` etc.
Filter via `RUST_LOG`. Logs go to stderr in a human-readable format
by default; structured JSON output gated by an env var for future CI
ingestion.

**Acceptance criteria.**

- `tracing = "0.1"` and `tracing-subscriber = { version = "0.3",
  features = ["env-filter", "fmt", "json"] }` in `src-tauri/
  Cargo.toml`.
- `crates/slic3r-ffi/Cargo.toml` also gains `tracing` (without
  `tracing-subscriber`) so the library crate can emit spans/events
  without imposing a subscriber.
- src-tauri's `run()` calls a subscriber init function early
  (before `tauri::Builder::default()`) that honors
  `RUST_LOG` (defaulting to `info`) and emits to stderr.
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

**Dependencies.** None. Independent of P0-1, P0-3.

**Out of scope.** Replacing libslic3r's own `boost::log` output (the
"Logging sink redirect" item in PRD §8.3) — that's a separate FFI
extension. Today libslic3r still goes to stderr directly via boost.

---

## P0-3 — Stub the `core/` module structure per PRD §8.2

**Scope.** Create the directory tree the PRD's architecture (§8.2)
calls for. Each module is an empty Rust submodule with a docstring
that names its responsibility and links to the PRD section that owns
its requirements. The point is to lock the module boundaries early
so subsequent phases can colocate work without bikeshedding the
layout.

**Acceptance criteria.**

`src-tauri/src/core/` contains:

```
core/mod.rs                    # umbrella module
core/cascade/mod.rs            # rule cascade resolver (PRD FR-CAS-1..13)
core/cascade_adapter/mod.rs    # logical → DynamicPrintConfig (FR-CAS-14..17)
core/project/mod.rs            # project model, plate-printer binding (FR-MP-*)
core/scene/mod.rs              # renderer-agnostic 3D scene state (FR-3D-7 / AD-8)
core/slice/mod.rs              # FFI orchestration, progress events (FR-SL-*)
core/gcode/mod.rs              # typed G-code model, parser, serializer (FR-GP-*)
core/threemf/mod.rs            # 3MF reader/writer
core/filament/mod.rs           # filament profile + sync (FR-FS-*)
core/plugin/mod.rs             # Lua host, hook dispatch (FR-PL-*)
core/printer/mod.rs            # driver-trait registry
core/printer/bambu/mod.rs      # Bambu MQTT (FR-BL-*)
core/printer/snapmaker/mod.rs  # Snapmaker HTTP (FR-SU-*)
```

- Each `mod.rs` has a `//!` docstring with one paragraph naming the
  responsibility plus a `//! See PRD §<n>` reference.
- `core` is declared in `src-tauri/src/lib.rs` as `pub mod core;`.
- The existing `slicer_*` Tauri commands move into
  `core/slice/mod.rs` (or wherever fits — slicer commands belong in
  the slice module). Re-exports keep the public command surface
  unchanged from the frontend's perspective.
- `cargo check -p n3o-slic3r` is clean.
- `cargo build -p n3o-slic3r --bin n3o-slic3r` still produces a
  working binary.

**Effort.** ~1 day.

**Dependencies.** None. Best done before any new functionality
lands so the homes for it are pre-decided.

**Out of scope.** Implementing any of the modules. They stay empty
(beyond the docstring) until their phase. The current
`slicer_options` / `slicer_info` commands stay functional but may
relocate to `core/cascade/mod.rs` (option introspection is cascade
territory) or `core/slice/mod.rs` (since they touch the FFI).

---

## P0-4 — Linux CI workflow

**Scope.** GitHub Actions workflow that builds the workspace and
runs the test suite on every push and PR. Caches the OrcaSlicer
deps tree because that's the long pole.

**Acceptance criteria.**

- `.github/workflows/build.yml` exists.
- Triggers: `push` to `main`, `pull_request` against `main`,
  `workflow_dispatch`.
- Runs on `ubuntu-latest`.
- Steps in order:
  1. Checkout with `submodules: recursive`.
  2. Install system deps (`gtk3`, `dbus`, `webkit2gtk`, `mesa`,
     `cmake`, `ninja`, GCC, pkgconf). Use `apt` directly.
  3. Cache `external/OrcaSlicer/deps/build/OrcaSlicer_dep/` keyed
     by the OrcaSlicer submodule SHA + the deps build script
     contents. Cache miss → run `./scripts/build.sh deps`.
  4. Cache `~/.cargo` and `target/` keyed by `Cargo.lock` +
     submodule SHA.
  5. Install Rust stable (`dtolnay/rust-toolchain@stable`).
  6. Install Node 20.
  7. `npm ci`.
  8. `cargo test --workspace --release` — builds the FFI .so via
     cmake, builds the Rust workspace, runs the 16 integration
     tests in `crates/slic3r-ffi/tests/api.rs`.
  9. `npm run build` — TS check + Vite build.
  10. `npx tsc --noEmit` on the renderer for good measure.
- A first run completes successfully on a fresh push.
- CI badge in `README.md` reflecting the latest status.

**Effort.** 1–2 days. Most of the time is cache tuning to keep the
deps build to cold runs only — without that, every CI run is ~17 min
on the deps step alone.

**Dependencies.** Blocked on pushing the repo to a remote
(currently local-only). The workflow file can be authored and
committed in advance; verification needs the remote.

**Out of scope.** Windows and macOS runners (PRD §3.2 explicit
non-goal for MVP). Release-artifact upload (flatpak build) — Phase 9.
GUI smoke tests with xvfb — also post-MVP.

---

## P0-5 — Phase 0 exit-criteria smoke

**Scope.** A concrete script and checklist that exercises Phase 0's
exit criteria end-to-end. Document it so the same smoke runs after
the libslic3r submodule bump, the cargo toolchain bump, etc.

**Acceptance criteria.**

- `docs/phase-0-smoke.md` (or appended section in
  `phase-0-tickets.md`) documents the smoke procedure:
  1. `git submodule update --init --recursive`
  2. `./scripts/build.sh deps` (skip if already built)
  3. `cargo test --workspace --release` — 16/16 tests pass.
  4. `cargo run -p slic3r-ffi --release --example introspect` —
     prints "OrcaSlicer libslic3r_ffi v0" and "total options: 737"
     (or N if upstream changed).
  5. `cargo run -p slic3r-ffi --release --example slice -- <test
     STL> /tmp/out.gcode` — produces a non-empty gcode file.
  6. `npm install && npm run tauri dev` — app window launches,
     header shows the libslic3r version + option count, slice form
     works against a known test 3MF.
- The smoke procedure runs cleanly from a clean checkout. Any
  divergence from the documented expected output is recorded as a
  bug or a documentation update.

**Effort.** Half a day, including running the procedure once to
confirm.

**Dependencies.** P0-1, P0-2, P0-3 complete. P0-4 is independent
(CI runs the same smoke).

**Out of scope.** Anything that touches printer hardware
(connectivity is Phase 7). Anything in the renderer beyond
"launches and displays version." Multi-printer workflows (Phase 5).

---

## Notes on what's *not* in Phase 0

Worth restating so future readers don't confuse phase boundaries:

- **Cascade resolver** — Phase 1.
- **3D viewport** — Phase 2.
- **End-to-end slice through a settings UI** — Phase 3.
- **Settings panel UI with cascade ladder** — Phase 4.
- **Plate-printer binding, multi-printer projects** — Phase 5.
- **G-code preview** — Phase 6.
- **Printer connectivity + filament sync** — Phase 7.
- **Plugin system** — Phase 8.
- **Flatpak + release prep** — Phase 9.

If a Phase 0 ticket starts pulling in any of the above, that's
scope creep. Cut the ticket back, or move it to the appropriate
phase.

## Phase 0.5 reminder

After Phase 0 closes, Phase 0.5 (~1 person-week) runs five
engine-validation spikes before Phase 1 commits to the cascade
design. Spike 4 (coEnums) is already done; the other four
(cascade adapter end-to-end, mixed-nozzle-size slice, A1 mini AMS
slice, platecycler portability) are real Phase 0.5 work. Tickets
for those should be created when Phase 0 wraps.
