# PR-7a-7 — Frontend printer state panel + send button

Status: ✅ landed.

## Scope cuts made at implementation time

Two deferrals discussed with the user during build-out, both
captured here so a follow-up ticket can pick them up cleanly:

- **Credentials persistence**: NOT shipped. Per user direction the
  "whole printer set up is currently misdesigned" — credentials
  live only in an in-memory cache (`credentialsCache.ts`) keyed
  by `printer_identity`, lost on app reload. No fields added to
  `core/project/model.rs`; nothing reaches the project `.3mf`.
  Memory item: `feedback_no_credentials_in_project_file.md`.
- **Auto-registration on plate binding**: NOT shipped. Spec said
  the MaterialBindingPanel should call `driver_register` /
  `driver_connect` on bind. Instead, the panel renders a "Connect
  printer" button that opens `PrinterCredentialsDialog`. Auto-
  registration is a clean follow-up once the printer-setup design
  is revisited.

## Other choices worth documenting

- The hook test (`useDriverStatus`) is not included — this repo's
  vitest config has no jsdom + no RTL + no `renderHook`, and the
  hook has no extractable pure logic. Matches the existing
  convention (e.g. `usePlateTabs.test.ts` only tests the pure
  projection). Component lifecycle is covered by visual + future
  Playwright smoke.
- Status events are pumped to the frontend via a per-driver tokio
  task spawned at `driver_register` time. The bridge owns the
  driver's `watch::Receiver<PrinterStatus>` and emits a Tauri
  `driver:status_update` event on every watch change. No polling
  in either direction.
- The .gcode → .gcode.3mf wrap happens server-side in
  `driver_send_plate` via PR-3-10's writer (`fixture_input` →
  `write_sliced_3mf`) as a stub; PR-7c-7 will replace it with the
  full sync-on-send pipeline (AMS bindings, project metadata).

**Scope.** First UI surface for the driver. Sits in the topbar
area near the Slice + Preview buttons. Shows the active plate's
bound printer's live status; lets the user send the most-recent
slice to that printer; surfaces AMS slot state inline; surfaces
pause/resume/stop affordances.

**Acceptance criteria.**

- New module `src/driver/`:
  - `invokes.ts` — Tauri invoke wrappers for the `driver_*`
    commands from PR-7a-1.
  - `useDriverStatus.ts` — React hook subscribing to
    `driver:status_update` events; returns the latest
    `PrinterStatus` for a given `driverId`.
  - `PrinterPanel.tsx` — the component.
  - `BambuAmsStrip.tsx` — sub-component rendering the 4-slot
    AMS state (colored chips per loaded spool).

- **Auto-registration**: when the active plate gets bound to a
  printer (Phase 5's MaterialBindingPanel triggers this), the
  app:
  1. Reads the printer's `(host, access_code, serial)` from the
     project state (lookup by `printer_identity`).
  2. If no credentials are persisted yet, opens a
     `PrinterCredentialsDialog` modal asking for host + access
     code.
  3. Calls `driver_register({kind: "Bambu", config: {host, access_code, serial}})`.
  4. Stores the returned `driver_id` in the plate's binding.
  5. Calls `driver_connect(driver_id)` immediately.

- **`PrinterCredentialsDialog`** (`src/driver/PrinterCredentialsDialog.tsx`):
  - Three text inputs (host, access code, serial — last
    pre-filled from the printer profile if known).
  - "Test connection" button → calls `driver_register` +
    `driver_connect` + tears down on success/failure with the
    error surfaced.
  - On success, persists the credentials into the project
    (Phase 5 project file).

- **`PrinterPanel`** layout (collapsed by default; expand
  reveals AMS strip + advanced state):
  - Status pill: connection state (Connected / Reconnecting /
    Disconnected) with color coding.
  - Job line: `<file_name> — Layer N/M — XX% — ETA HH:MM:SS`
    when printing; "Idle" otherwise.
  - Temps line: `Nozzle TTT/SET · Bed TTT/SET`.
  - Send button:
    - Enabled when: driver is Connected AND active plate has
      a recent slice AND state is IDLE.
    - Disabled tooltip explains why (no slice / not connected
      / printing).
    - On click: reads the plate's last-sliced
      `.gcode.3mf` bytes (built by PR-7c-7's sync-on-send
      hook; stub to "use raw gcode wrapped via PR-3-10" until
      7c lands), calls `driver_send`, surfaces the returned
      `SendHandle.id` in the panel.
  - **Dry-run send button** (next to Send): same enablement
    rules as Send, but calls `driver_dry_send` instead. The
    backend (`core/driver/dryrun.rs`) neuters the bundle —
    strips E values from G0/G1/G2/G3 motion lines and comments
    out M104/M109/M140/M190 heater commands — before forwarding
    to the driver. The printer goes through every XY motion
    without heating or extruding; intended as the first send
    against a freshly-paired printer to confirm the toolpath
    without risking the bed. Visually distinct from the Send
    button (e.g., outlined, smaller, with a "no-heat" badge or
    similar affordance) — accidentally clicking dry-run when the
    user meant Send is harmless; the reverse is a destroyed bed.
  - Command buttons (Pause / Resume / Stop):
    - Visible only when state matches a valid transition
      (Pause on RUNNING, Resume on PAUSE, Stop on RUNNING|PAUSE).
    - On Stop: confirmation dialog ("Stop the current print?
      This cannot be undone.").
  - AMS strip (collapsible, default open when 1+ slot loaded):
    `BambuAmsStrip` renders 4 colored chips. Empty slots
    render dashed-outline. Hover shows `tray_type` + color hex.
    The active slot has a ring around it.

- **`BambuAmsStrip`** props:
  ```ts
  interface BambuAmsStripProps {
    ams: AmsState | null;
  }
  ```
  Renders nothing when `ams` is null.

- **App.tsx integration**:
  - Mount `PrinterPanel` in the topbar between the SlicePanel
    and the Preview button.
  - Pass the active plate's bound `driver_id` (or null).
  - When `driver_id` is null, panel renders a "Bind a printer"
    placeholder linking to the MaterialBindingPanel.

- Tests:
  - `BambuAmsStrip.test.tsx` — render with various AMS shapes,
    assert the right number of chips + the right colors.
  - `useDriverStatus.test.ts` — mock the Tauri event channel,
    assert the hook returns the latest snapshot + updates on
    incoming events.
  - `PrinterCredentialsDialog.test.tsx` — happy path + failed
    auth.

**Effort.** ~2 days. Most of it is the credentials modal +
the auto-registration wiring; the panel itself is a thin
display layer over the status hook.

**Dependencies.** PR-7a-1..-6, PR-5-6 (MaterialBindingPanel
triggers binding), PR-5-8 (project file persistence for
credentials).

**Out of scope.**

- Multi-printer simultaneous panels — only the active plate's
  printer shows. Status of disconnected drivers is still
  polled but not surfaced; multi-panel UX is Phase 9.
- Printer discovery (mDNS / Bonjour) — manual host entry only
  for MVP.
- Save / send arbitrary file from disk — only the plate's own
  slice is sendable. A free-form send affordance is Phase 9.
