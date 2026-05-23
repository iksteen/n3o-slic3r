# PR-5-10 — Autosave + recovery on launch

Status: ❌ open.

**Scope.** Every 30 seconds, snapshot the current project
state to an autosave file. On app launch, check for a
newer-than-last-explicit-save autosave file and offer
recovery via a startup dialog.

Owns the autosave half of FR-MP-4 (project file format) +
the recovery exit-criterion language.

**Acceptance criteria.**

- Backend:
  - New `core/project/autosave.rs` with an `AutosaveWorker`
    that owns a `tokio::time::interval` (or `std::thread`
    + `Condvar` if we want to keep tokio off the
    dependency graph for now).
  - Worker runs every 30 s. Reads the Tauri-managed
    `Project` state (locked via `Mutex`), copies it,
    writes to the autosave file outside the lock so the
    UI doesn't block during disk I/O.
  - Autosave path:
    `~/.local/share/n3o-slic3r/autosave/<project-uuid>.3mf`
    where `<project-uuid>` is generated per project on
    creation (stored in `Project.uuid`, added to
    `PR-5-1`'s schema as a follow-up).
  - On graceful shutdown (Tauri's `on_window_event` hook),
    flush a final autosave + drop the file if the project
    has no unsaved changes.

- Recovery on launch:
  - On app start, scan the autosave directory for files
    newer than the last explicit save (mtime > saved-file
    mtime).
  - If any are found, surface a startup dialog: "Recover
    unsaved project from {timestamp}?" with options
    *Recover* / *Discard* / *Keep* (keep = leave the
    file in place but don't load it; lets the user
    inspect via the file manager).
  - "Recover" loads the autosave via PR-5-8's
    `project_load`.

- New Tauri commands:
  - `project_autosave_enable() -> Result<(), String>` —
    starts the worker if not already running.
  - `project_autosave_disable() -> Result<(), String>` —
    stops the worker.
  - `project_autosave_list() -> Vec<AutosaveEntry>` —
    returns the recovery candidates the startup dialog
    consumes.
  - `project_autosave_drop(uuid: String) -> Result<(), String>`
    — deletes a specific autosave file (the "Discard"
    button).

- Frontend:
  - `src/project/AutosaveRecoveryDialog.tsx` — the
    startup modal that lists recoverable autosaves with
    *Recover* / *Discard* / *Keep* buttons.
  - Mounts at the App.tsx root before the main UI; gates
    the rest of the app until the user decides.

- Tests:
  - Autosave file is written ~30s after a project change
    (use a short interval in tests).
  - Recovery candidates surface only when the autosave
    mtime > last-explicit-save mtime.
  - "Discard" deletes the file; "Keep" leaves it.
  - Multiple n3o-slic3r instances: each owns a distinct
    project uuid → no autosave collision.

**Effort.** ~2 days. The worker is small; the recovery
dialog + the startup-gate flow + the multi-instance
safety is the bulk.

**Dependencies.** PR-5-8 (project_save / project_load).
PR-5-1 needs a `Project.uuid` field added if not already
present (suggest adding to PR-5-1 directly so the schema
is settled before PR-5-10 lands).

**Out of scope.** Cloud-sync autosave. Per-project autosave
intervals (Phase 9 if anyone asks). Background-thread
priority tuning (the worker is I/O-bound; default
scheduling is fine).

**Cut candidate.** The startup recovery dialog (~1 day per
Execution Plan §7 cut list) → autosave still runs but
there's no automatic recovery surface. Users would have
to find the file manually via the file manager. Cut if
shipping date pressure hits; document the autosave
location in `docs/phase-5-smoke.md` so the manual
workaround is discoverable.
