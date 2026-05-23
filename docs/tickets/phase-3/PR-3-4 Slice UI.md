# PR-3-4 — Slice button + progress bar + summary panel

Status: ✅ shipped — `src/slice/{types,reducer,useSliceJob,SlicePanel}.tsx` with the panel wired into `App.tsx`'s header. `useSliceJob` subscribes to the six `slice:*` events, runs them through a pure reducer (vitest-covered), exposes `{status, percent, stage, summaries, error}` plus `start()` / `cancel()` / `reset()`. Backend gained `slice_start_default_a1mini` (Tauri command) that bundles the cascade (embedded via `include_str!` of `profiles/cascades/bambu-a1-mini-default.toml`) + canonical A1 mini / Textured PEI / Generic PLA context — Phase 4's profile UI will replace this with a project-state-driven call to `slice_start_job` directly. The panel guards Slice until a model file has been picked; outputs land at `/tmp/n3o-slice-<stamp>/plate_1.gcode`. Reconnect path uses `slice_status` keyed by a localStorage-cached job id. Per-plate summary cards show formatted time, aggregated filament use (g + m), and layer count. Failure path renders `sliceErrorMessage()` on the typed `SliceError`. Outstanding: manual browser smoke not yet performed in this session.

**Scope.** Frontend surface for the slice loop: a button next to the
viewport toolbar, a progress bar that subscribes to PR-3-2's events,
and a summary panel that pops up on completion.

Phase 4 will restyle the whole settings + slice + plate UI; this
ticket ships the minimum-viable surface to drive the slice loop and
verify the exit smoke (PR-3-12).

**Acceptance criteria.**

- `src/slice/` directory (new, sibling to `src/viewport/`):
  - `useSliceJob.ts`: React hook that owns subscription to
    `slice:*` events, exposes `{ status, percent, stage,
    perPlate, summaries }` reactive state, and `start()` /
    `cancel()` actions wrapping the Tauri commands.
  - `SlicePanel.tsx`: button + progress bar + per-plate
    summaries. Pops a toast on slice failure with the typed
    `SliceError` message (PR-3-3) inlined.

- Wire into `App.tsx`: SlicePanel sits in the header next to the
  Debug toggle (Phase 2 layout). Clicking *Slice* with an active
  printer + at least one visible scene object starts a single-
  plate job to `<tmp>/n3o-slice-<job_id>/plate_1.gcode`.

- Per-plate summary cards render `PlateSummary` from PR-3-3:
  - "12m 34s" formatted estimate
  - "PLA: 4.2g · 1.4m" per extruder
  - "247 layers, 184k G-code lines"

- Cancel button while a job is running. Disabled when no job in
  flight.

- Reconnect path: on mount, `useSliceJob` calls `slice_status` to
  rebuild progress state if a slice was running when the renderer
  reloaded.

- vitest happy-path: `useSliceJob` reducer is a pure function over
  the event stream; a unit test feeds a representative event
  sequence (`plate_started → progress (×4) → plate_finished →
  job_finished`) and asserts the final state has `status:
  "complete"`, `percent: 100`, and a populated `summaries` array.

**Effort.** ~1 day. The reducer + Tauri-listen plumbing is the bulk;
the visuals follow Phase 2's neutral-900 dark style for now.

**Dependencies.** PR-3-2 (events), PR-3-3 (summary + typed errors).

**Out of scope.** Multi-plate UI (Phase 5). Per-object slice (the
slice always operates per-plate). Settings overrides directly from
the SlicePanel (Phase 4 ships the settings UI; for now the cascade
just resolves from the bundled A1-mini defaults).
