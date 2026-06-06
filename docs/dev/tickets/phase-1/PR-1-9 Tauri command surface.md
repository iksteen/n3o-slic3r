# PR-1-9 — Tauri command surface for cascade + adapter

Status: ⚠️ partial. `src-tauri/src/core/cascade/commands.rs` ships `CascadeRegistry` in Tauri State + four commands: `cascade_load`, `cascade_resolve`, `cascade_trace`, `cascade_context_dimensions`. `ContextJson` carries the printer/plate/filaments/overrides shape the frontend builds per call. `cascade_apply` (returning a ConfigHandle) is deferred — the existing `slicer_slice` command takes a model path and config directly; cascade-into-Config wiring lands when `slicer_slice` grows a `from_cascade_handle` variant. 3 unit tests cover load + resolve + trace via the registry, the canonical dimensions list, and the unknown-handle path.

**Scope.** The frontend-facing commands the Phase 4 Settings UI
(and the Phase 1 CLI test harness in PR-1-11) need to drive the
resolver / adapter. All commands live in `core/cascade/` and
`core/cascade_adapter/`, exposed via `tauri::command` macros and
registered in `src-tauri/src/lib.rs`.

**Acceptance criteria.**

The full command list:

- `cascade_load(paths: Vec<String>) -> CascadeHandle` — loads one
  or more cascade files. Returns an opaque handle the frontend
  uses for subsequent calls. Errors are surfaced with file:line
  info via Tauri's standard error channel.

- `cascade_resolve(handle: CascadeHandle, ctx: ContextJson) ->
  ResolvedJson` — resolves a cascade against a serialized
  `Context` (JSON shape mirrors PR-1-7's `Context`). Returns the
  flat `BTreeMap<String, ResolvedValue>` from PR-1-3 / PR-1-4.

- `cascade_trace(handle: CascadeHandle, ctx: ContextJson, key:
  String) -> Option<TraceJson>` — returns the structured trace
  from PR-1-5 for a single setting. The "why is X = 55?" surface.

- `cascade_apply(handle: CascadeHandle, ctx: ContextJson) ->
  ConfigHandle` — runs the resolver + adapter and returns a
  handle to the resulting `slic3r_ffi::Config`. Phase 3's slice
  command consumes this handle.

- `cascade_context_dimensions() -> Vec<DimensionDescriptor>` —
  enumerates the dotted context keys the cascade accepts
  (`printer.model`, `filament.type`, `plate.type`, ...) plus their
  valid values (drawn from the loaded printer / plate / filament
  profiles). Drives Phase 4's predicate-builder UI.

- All commands are `#[tracing::instrument]` per Phase 0's
  convention.

- Handle lifecycle: `CascadeHandle` and `ConfigHandle` are stored
  in a Tauri `State<Arc<Mutex<HandleRegistry>>>`. Reloading a
  cascade invalidates prior handles (commands using stale handles
  return `Err(StaleHandle)`).

- Tests: each command has at least one happy-path test against
  the reference cascade from PR-1-8 (`cascade_resolve` against
  A1 mini + PEI + PLA returns expected `bed_temp = 65`,
  `cascade_trace` returns a 3-rule trace, etc.). End-to-end
  test via `tauri::test` if the test infra supports it; else
  call the Rust functions directly with a fake `State`.

**Effort.** ~2 days.

**Dependencies.** PR-1-3, PR-1-4, PR-1-5, PR-1-6, PR-1-7, PR-1-8
all complete.

**Out of scope.** Hot-reload of cascade files when they change on
disk — Phase 4 UI work. Per-extruder override commands (`set this
slot's filament to PETG`) — Phase 5 multi-printer work.
