# PR-7c-3 — Filament state panel UI + manual override + badge

Status: ❌ open.

**Scope.** UI surface for the FilamentState model. Per-printer
panel showing the live loadout, with manual-override controls
that visibly distinguish overrides from printer-reported values.

**Acceptance criteria.**

- New module `src/filament/`:
  - `invokes.ts` — Tauri invoke wrappers for the
    `filament_state_*` commands from PR-7c-2.
  - `useFilamentState.ts` — React hook subscribing to
    `driver:filament_updated`; returns the latest
    `PrinterFilamentLoadout` for a given `printer_identity`.
  - `FilamentStatePanel.tsx` — the component.

- **`FilamentStatePanel`** layout (compact, mountable inside
  the existing PrinterPanel under the AMS/toolhead strip):
  - Header: "Loaded filaments" + last-updated timestamp +
    refresh button.
  - One row per slot:
    - Color swatch from `effective().color` (override-
      preferred, then reported).
    - Filament name from `effective().name` (or "Empty" /
      "Unknown").
    - **Override badge** (orange chip "Override") when
      `override` is set.
    - Override dropdown: picks from
      `filament_library_list()`. Selecting an entry calls
      `filament_state_set_override`. A "Reset" link clears
      the override.

- **Color resolution rule**: when both `override` and
  `reported` exist, the swatch shows the OVERRIDE's color but
  with a small corner-triangle indicator showing the reported
  color (so the user knows what's actually loaded).

- **Empty-slot handling**: dashed outline + "Empty" label.
  Override dropdown still works (user can pre-bind a profile
  to an empty slot for planning purposes).

- **Mount inside `PrinterPanel`** (PR-7a-7 / PR-7b-8):
  - Below the AMS strip / toolhead strip, collapsible
    section "Filament details" — when expanded, mounts
    `FilamentStatePanel`.

- Tests:
  - `useFilamentState.test.ts` — mocks the event channel,
    asserts the hook surfaces fresh state.
  - `FilamentStatePanel.test.tsx` — render with: all empty
    / all reported / mixed with overrides / refresh click.
  - **Snapshot test for the override-badge styling**.

**Effort.** ~1.5 days.

**Dependencies.** PR-7c-2 (FilamentState backend), PR-7a-7
(PrinterPanel scaffold), PR-7c-1 (library list for the
dropdown).

**Out of scope.**

- Inline custom-profile editor — out of MVP scope per
  PR-7c-1.
- Filament temperature / cooling tuning UI from this panel
  — that lives in the SettingsPanel.
- Spool-management features (run-out detection, low-spool
  alerts, etc.) — Phase 9+.
