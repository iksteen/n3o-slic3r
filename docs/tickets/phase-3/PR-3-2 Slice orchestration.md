# PR-3-2 — Slice orchestration on a worker thread

Status: ❌ open.

**Scope.** Rewrite the Phase 0 `slicer_slice` command into a
proper orchestrator: runs on a worker thread, slices each plate
sequentially, streams progress events to the UI via Tauri, accepts a
cancellation handle.

Owns PRD FR-SL-1 (per-plate slice) and FR-SL-2 (off-UI-thread,
event-based progress). Lives in `core/slice/`.

**Acceptance criteria.**

- New `core/slice/orchestrator.rs`:
  - `pub struct SliceJob { pub job_id: u64, pub plate_ids: Vec<u32>,
    pub output_dir: PathBuf, pub cascade_handle: CascadeHandle, ... }`
    — what the UI fires off.
  - `pub fn start_slice_job(scene, job) -> Result<SliceHandle,
    SliceStartError>` — validates input, spawns the worker thread,
    returns the handle the UI uses to cancel.
  - Worker thread iterates plates: for each, build the libslic3r
    `Model` + `Config` from the scene + the active cascade
    (resolver + adapter from PR-1-*), call into the FFI's slice
    entrypoint, emit `slice:plate_progress` events as the FFI
    callback (PR-3-1) fires.
  - Per-plate output written to `<output_dir>/plate_<N>.gcode`.

- Tauri commands:
  - `slice_start_job(job: SliceJob) -> JobId`
  - `slice_cancel(job_id: JobId)` — flips a cancellation flag the
    worker polls between stages; surfaces a `slice:cancelled`
    event on next callback.
  - `slice_status(job_id: JobId) -> SliceStatus` — for the UI to
    rebuild on reconnect, same pattern as `scene_snapshot`.

- Events (carries `job_id` on every variant):
  - `slice:plate_started { job_id, plate_id }`
  - `slice:plate_progress { job_id, plate_id, percent, stage }`
  - `slice:plate_finished { job_id, plate_id, output_path,
    summary }` — `summary` is a stub for PR-3-3.
  - `slice:job_finished { job_id }`
  - `slice:job_failed { job_id, plate_id, error }` — the
    plate_id where the failure happened so the UI can highlight.
  - `slice:cancelled { job_id, plate_id_in_progress }`.

- Progress emission is rate-limited: the FFI callback can fire
  hundreds of times per second on a fast slice. The orchestrator
  collapses to one event every 50 ms per plate to keep the Tauri
  event channel from saturating. Stage *changes* always emit
  immediately so the UI feels responsive on phase boundaries.

- Cancellation: PR-3-1 doesn't expose a libslic3r-level cancel
  hook for free; we cooperate by checking a `should_cancel:
  Arc<AtomicBool>` between plates. Per-plate-mid-slice cancel is a
  follow-up (libslic3r side; see `docs/libslic3r-workarounds.md`).

- Tests:
  - Integration test under `src-tauri/tests/slice_orchestrator.rs`:
    feed a small scene with two single-cube plates, drive the
    full orchestration, assert each plate emits started → at least
    one progress → finished, and that `<output_dir>/plate_1.gcode`
    + `plate_2.gcode` both exist and parse as G-code (using
    PR-3-6's parser when it's ready; pre-parser, just `grep "M104"`).
  - Cancellation test: start a 4-plate job, cancel after the
    first plate finishes, assert the second plate emits
    `slice:cancelled` and plates 3-4 never start.

**Effort.** ~3 days. Worker-thread plumbing + Tauri event wiring is
mechanical; the friction is integrating with the cascade resolver
+ adapter so each plate's `Config` is built from the right context
(active printer + filament loadout per plate). PR-1-7's
`SlicingContext` is the right input shape.

**Dependencies.** PR-3-1 (progress callback). PR-1-* (cascade
resolver + adapter; already shipped). The Phase 2 scene state +
multi-plate readiness (Phase 5 lands proper multi-plate; Phase 3
operates on the single-plate model with `plate_ids: vec![1]` in
practice).

**Out of scope.** Multi-plate UI (Phase 5). Per-plate-mid-slice
cancel (libslic3r-side hook). Distributed/remote slicing —
post-MVP.
