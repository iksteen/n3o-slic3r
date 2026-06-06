# PR-6-2 — `slice_active_plate` Tauri command

Status: ✅ shipped.

**Scope.** Replace `slice_start_default_a1mini` (the path-based
"slice this file" command) with `slice_active_plate` (the
state-based "slice the live scene's active plate" command).
Plumbs PR-6-1's input builder through the Tauri command layer
+ orchestrator, owns the temp-file lifecycle, and emits the
existing `SliceEvent::*` stream verbatim so the existing
`useSliceJob` reducer continues to work.

**Acceptance criteria.**

- New Tauri command in `src-tauri/src/core/slice/commands.rs`:

  ```rust
  #[tauri::command]
  pub fn slice_active_plate(
      plate_id: Option<PlateId>,
      app_handle: AppHandle,
      project: State<Arc<Mutex<Project>>>,
      jobs: State<JobRegistry>,
      cascades: State<Mutex<CascadeRegistry>>,
  ) -> Result<JobId, String>;
  ```

  - `plate_id: None` defaults to the project's active plate.
  - Reads the project under the existing `Mutex<Project>`
    state, calls `build_slice_input(&project, plate_id)`
    from PR-6-1, then `start_slice_job(input, …)` from the
    Phase 3 orchestrator.
  - Returns the `JobId` the same way the old command did.

- **Temp-file cleanup.** The command captures the temp path
  returned by PR-6-1 and registers a cleanup hook on the
  job's terminal events:
  - `SliceEvent::JobFinished` → delete temp file
  - `SliceEvent::JobFailed` → delete temp file (keep with a
    tracing warning if the cleanup itself fails)
  - `SliceEvent::JobCancelled` → delete temp file
  - Cleanup uses `std::fs::remove_file`; missing-file is a
    debug-log no-op, not an error (the OS may have GC'd
    temp dirs).
  - Implementation: extend the orchestrator's terminal-event
    handler with an optional `Box<dyn FnOnce() + Send>` hook
    the command supplies, or a `Vec<PathBuf>` of "delete me
    on terminate" entries attached to the `JobHandle`. Pick
    whichever the existing structure accommodates more
    naturally.

- **`slice_start_default_a1mini` is removed.** Drop the
  command from `default_a1mini.rs` + the lib.rs registration.
  Keep the `canonical_printer / canonical_plate /
  canonical_filament` helpers (PR-6-1 may reuse them as
  fallbacks, or they get deleted in this ticket — confirm
  during impl).

- **Error mapping.** `SliceInputError` from PR-6-1 maps to
  user-visible error strings:
  - `UnknownPlate(_)` → "active plate is gone; pick another"
  - `UnboundPrinter { … }` → "bind a printer to the plate first"
  - `EmptyScene { … }` → "add an object before slicing"
  - `CascadeContextUnavailable { message }` → message verbatim
  - `TempWrite { source, … }` → "couldn't write slice input: …"

- Tests (`src-tauri/src/core/slice/commands.rs` integration
  test or sibling):
  - **Happy path:** drive the command with a 1-plate, 1-cube,
    A1-mini-bound project; assert the returned `JobId` is
    registered in `JobRegistry`, the temp file existed during
    the job, and the temp file is gone after `JobFinished`.
  - **Unbound printer** returns the mapped error string.
  - **Empty plate** returns the mapped error string.
  - **Cancel mid-flight** still cleans up the temp file
    (use a test that cancels before libslic3r finishes — may
    require a long-running fixture or a fake orchestrator).

**Effort.** ~1 day. Most of the work is the temp-file
lifecycle plumbing; the command body itself is a thin shim
over PR-6-1.

**Dependencies.** PR-6-1, PR-5 Mutex<Project> state, Phase 3
orchestrator (`start_slice_job`, `SliceEvent` terminal
events).

**Out of scope.**

- Multi-plate "slice all plates" loop (Phase 7 polish; or a
  quick follow-up if the user wants it).
- The frontend Slice button rewire (PR-6-3).
- Modifying the orchestrator's slice mechanics — only the
  terminal-event handler grows a cleanup hook.

**Cut candidate.** None.
