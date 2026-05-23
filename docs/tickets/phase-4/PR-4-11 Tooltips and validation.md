# PR-4-11 — Tooltips + inline validation

Status: ❌ open.

**Scope.** Two row-level affordances that round out the settings UX:

1. **Tooltips** combining libslic3r's tooltip text with an
   authored "why this matters" annotation layer. The first ~30
   annotations ship with PR-4-12; PR-4-11 ships the rendering
   surface.
2. **Inline validation** against libslic3r's `config_validate`.
   Invalid input gets a red border + message; an invalid project
   cannot be sliced (the slice button — still PR-3-4's
   `SlicePanel` — disables when any setting is invalid).

Owns FR-UI-6 (tooltips) and FR-UI-5 (validation).

**Acceptance criteria.**

- Tooltip rendering (`src/settings/tooltip/SettingTooltip.tsx`):
  - Hover on the row label opens the tooltip with a 400 ms delay
    (matching the cascade ladder's open delay but on the label,
    not the row — distinct hover regions so they don't conflict).
  - Renders two sections stacked:
    - **libslic3r tooltip** — the option's `tooltip` field from
      its `ConfigOptionDef`. (Needs PR-4-1 to surface this on
      `OptionSummary` — currently the FFI returns it but the
      summary doesn't carry it; add `tooltip: Option<String>`.)
    - **Why this matters** — the authored annotation (PR-4-12
      ships the first 30). Rendered with a small "💡 tip"
      heading and a darker background to visually distinguish
      from the libslic3r text.
  - When no "why this matters" annotation exists for the key,
    only the libslic3r tooltip renders.

- Inline validation (`src/settings/validation/validate.ts`):
  - On every edit-commit (input blur / Enter), the panel calls
    a new Tauri command `slicer_validate_option(key, value)`
    that runs libslic3r's per-option validate routine
    (`config_validate` accepts a single-option dict — confirm
    the FFI exposes this; add it if not).
  - On validation failure, the row's `error` prop (reserved by
    PR-4-2's `<Field />` wrapper) gets the error message. The
    input renders with a red border + the message inline below
    the input.
  - Cross-option validation (e.g. "skirt_height must be ≤
    print_height") fires on the **whole config** when the user
    tries to slice — Phase 3's `SliceError::InvalidConfig`
    already surfaces these. PR-4-11 wires the SliceError back
    into the panel's row error state by matching
    `error.setting_key` to a row.

- Slice button gate:
  - The slice button in `SlicePanel` (PR-3-4) reads the
    panel-level error count. When > 0, the button is disabled
    with a tooltip "fix N invalid settings before slicing."
  - Wiring: a new shared store (`src/settings/validity.ts`)
    holds the current error map; both panels subscribe.

- Smoke check:
  - Type `-5` into `layer_height` → red border, message "must be
    > 0".
  - Slice button shows "1 invalid setting" tooltip and is
    disabled.
  - Fix the value → border clears, slice button enables.
  - Hover the row label for an annotated setting (e.g.
    `layer_height` — one of the first 30) → tooltip shows both
    libslic3r text and "💡 tip" section.

- vitest:
  - SettingTooltip renders both sections when both are provided;
    only libslic3r section when no annotation.
  - Validation failure surfaces in the row's `error` prop.
  - Validity store: pushing an error from key K bumps the panel
    error count; clearing K decrements.

**Effort.** ~2.5 days. Tooltip surface is half a day; validation
plumbing (FFI surface check + Tauri command + per-row error
state + slice-button gate) is ~2 days.

**Dependencies.** PR-4-1 (tooltip field on OptionSummary, validate
command), PR-4-2 (Field wrapper's `error` prop), PR-4-4 (panel
scaffold), PR-3-4 (SlicePanel's slice button — needs the gate
wired in).

**Out of scope.** "Auto-fix" suggestions for validation errors
("enable supports for this overhang" → Phase 4-stretch / Phase 9).
Cross-printer validation comparisons (Phase 5).

**Cut candidate.** The "💡 tip" / "why this matters" rendering is
gated by PR-4-12 shipping annotations. If PR-4-12 is cut, this
ticket gracefully degrades to libslic3r-tooltip-only.
