# PR-6-15 — App preview/3D mode toggle + topbar wiring

Status: ❌ open.

**Scope.** Top-level `mode: "scene" | "preview"` state in
App.tsx. Topbar `Preview [P]` button (per design mockup) +
keyboard shortcut toggle the mode. The viewport DOM region
swaps between `<ViewportCanvas/>` and `<GcodePreview/>`. The
settings panel hides in preview mode. Slice-job-completed
events auto-switch to preview mode and load the just-sliced
G-code.

**Acceptance criteria.**

- **`App.tsx` extension:**
  - New state: `const [mode, setMode] = useState<"scene" |
    "preview">("scene")`.
  - New state: `const [previewHandle, setPreviewHandle] =
    useState<PreviewHandle | null>(null)`.
  - Conditional render in the viewport region:
    - `mode === "scene"` → `<ViewportCanvas/>` (existing).
    - `mode === "preview"` → `<GcodePreview handle={previewHandle} …/>`.
  - Settings panel renders only when `mode === "scene"`.
    Preview's stats panels (PR-6-12) take the same column
    when `mode === "preview"`.

- **Topbar `Preview [P]` button:**
  - Already present in `docs/design/TopBar.jsx` — port to
    `src/components/TopBar.tsx` (or wherever the topbar
    lives post-PR-5-9).
  - Clicking toggles `mode`. Keyboard `P` (when no input is
    focused) also toggles. Tooltip: "Toggle G-code preview
    (P)".
  - Visual state: highlighted with `--accent` when in
    preview mode.
  - Disabled when `previewHandle === null` (no G-code
    loaded yet). Tooltip in disabled state: "Slice the
    active plate first".

- **Auto-switch on slice completion:**
  - Subscribe to the `slice:job_finished` Tauri event in
    App.tsx.
  - On event: invoke `preview_load(event.payload.output_path)`
    → set `previewHandle` → set `mode = "preview"`.
  - If the user manually toggled out of preview before the
    slice finished, don't yank them back — gated by a
    `userToggledOutOfPreview` ref. (Better UX than
    surprise switching.)
  - On `slice:job_failed` / `slice:job_cancelled`: don't
    auto-switch; surface the error normally.

- **Plate-tab interaction:** switching plates while in
  preview mode loads that plate's last-sliced G-code (if
  any) via `preview_load`. The preview registry indexes by
  plate id internally to support this; PR-6-7's
  `LoadedPreview` gets an optional `plate_id` field.

- **Empty state:** preview mode with `previewHandle === null`
  renders a centered placeholder: "Slice a plate to preview
  the G-code, or drag a .gcode file here." The drag-drop
  zone (PR-6-14) accepts files in this state.

- **Camera reset on mode swap:** the preview's camera
  re-defaults each time `mode` flips to `"preview"` (no
  persistence — Phase 9 polish).

- **Handle cleanup:** when switching plates or loading a
  new G-code, the previous `previewHandle` is dropped via
  `preview_drop` (PR-6-7) to free memory.

- Tests:
  - Mode-toggle button updates state correctly.
  - `P` keyboard shortcut toggles (with input-focus guard).
  - Auto-switch fires on `slice:job_finished`.
  - User-manual-toggle-out-of-preview blocks auto-switch.
  - Plate-tab change with cached preview loads the cached
    handle; with no cache, lands in empty state.

**Effort.** ~1.5 days. Mostly App.tsx integration + the
event subscription / auto-switch logic.

**Dependencies.** PR-6-3 (Slice button now lives in the
right flow), PR-6-7 (preview commands), PR-6-8 (renderer
mounts), PR-6-12 (stats panels take the panel column),
PR-6-14 (drop zone for empty state), PR-5-9 (existing
App.tsx integration), Phase 3 `SliceEvent::JobFinished` +
its Tauri event name.

**Out of scope.**

- Multi-pane view (3D + preview side-by-side) — post-MVP.
- Per-plate preview state persistence across reloads —
  Phase 9.
- "Refresh preview after edit" affordance — preview is
  read-only over a single G-code; edits → re-slice → new
  preview.

**Cut candidate.** Auto-switch-after-slice → save ~0.5
days. User would manually click the Preview button after
slicing. Hurts the "slice and immediately see what came
out" flow significantly. **Not recommended** unless the
phase is way over budget.
