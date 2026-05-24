# PR-6-3 — Frontend Slice button rewire

Status: ✅ shipped.

**Scope.** Drop the file-picker from `SlicePanel.tsx`. The
Slice button now just calls `slice_active_plate()` against
the live scene. Active-plate context comes from the
`useProjectSession` snapshot. Disabled when slicing wouldn't
succeed.

**Acceptance criteria.**

- `src/slice/SlicePanel.tsx`:
  - Remove the file-picker (`openDialog` + `modelPath`
    state, `pickModel` handler, the "pick model" button).
  - Replace the slice handler with `await
    invoke<JobId>("slice_active_plate", { plateId: null })`
    (null → backend uses active plate).
  - Output dir scheme stays as it was for now: per-job
    timestamped temp dir under `/tmp/`. (Future: derive
    output dir from project's `source_path` when the project
    is saved; that's a Phase 6 polish, not part of this
    ticket.)

- `src/slice/useSliceJob.ts`:
  - `start()` no longer takes `modelPath` — signature
    collapses to `start(outputDir: string)` or even
    `start()` with the temp-dir derivation moved into the
    hook. Pick the simpler shape; the panel only needs to
    say "go".

- **Disabled state.** The Slice button greys out + shows a
  tooltip when slicing can't proceed:
  - No active plate (snapshot loading / empty project)
  - Active plate has no objects (`plate.objects.length === 0`)
  - Active plate has no printer (`plate.printer == null`)
  - Job already running (`state.status === "running" ||
    "starting" || "cancelling"`)

- Tests (`src/slice/__test__/SlicePanel.test.tsx` or extend
  the existing reducer tests in `__test__/reducer.test.ts`):
  - Button renders "disabled" when active plate has no
    objects.
  - Button renders "disabled" when active plate has no
    printer.
  - Clicking the enabled button invokes `slice_active_plate`
    with `{ plateId: null }` (mock the invoke; assert call
    args).
  - Error path: backend rejects → error renders in the
    panel's startError region.

- **Drop dead code:**
  - `import { open as openDialog } from "@tauri-apps/plugin-dialog"`
    is gone from SlicePanel.
  - The `pickModel / basename / setModelPath` symbols are
    gone.
  - The "Slice the picked file" copy is replaced with
    "Slice plate {N}" or just "Slice".

- **Visual sanity:** the panel shrinks (no second button).
  PR-5-9's topbar layout should still look right; spot-check
  in `npm run tauri dev`.

**Effort.** ~0.5 days. Pure deletion + a small disabled-state
gate. Dead-code removal makes the diff net-negative.

**Dependencies.** PR-6-2 (`slice_active_plate` command),
PR-5-9 (App.tsx wiring + `useProjectSession`).

**Out of scope.**

- Removing the `tauri-plugin-dialog` Cargo/npm dependency —
  the file-open dialog is still used by mesh import elsewhere
  (`scene_object_add_from_primitive` adjacent flows).
- "Slice all plates" button — Phase 7 polish.
- Output-dir-from-project-path — Phase 6 polish.
- Topbar Slice button (TopBar.jsx mockup shows one) —
  PR-6-15's App-mode-toggle ticket may move the Slice button
  to the topbar; this ticket keeps it where it is.

**Cut candidate.** None.
