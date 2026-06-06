# PR-4-10 — Show-modified filter

Status: ✅ shipped — a "show modified only" toggle on the settings panel
(`SettingsPanel.tsx`), alongside the Simple/Advanced/Expert mode selector.
"Modified" = overridden at the active editing layer (a project override on the
Project tab; the selected object's override on the Object tab). FR-CAS-10.

> **Rescoped 2026-06-06.** The originally-planned **multi-baseline diff**
> (*vs printer default*, *vs last save*, *defaults + filament*, *vs another
> project plate*) was replaced by a single "what have I changed" view: a
> show-modified toggle. The last-save / another-plate / defaults+filament
> baselines were dropped — they added baseline-tracking plumbing (in-memory
> save snapshots, printer-only re-resolves, multi-plate compares) for a power-
> user convenience the override-count badges + cascade ladder already largely
> cover. The old `src/settings/diff.ts` (`computeDiff` / `passesDiff` /
> `DiffMode` + tests) was removed.

## Behavior

- **Toggle** next to the search field: off → the panel filters by mode +
  search as usual; on → only settings modified at the active layer are shown.
  The toggle carries a live count of modified settings and is disabled when
  there are none.
- **Modified settings are never hidden by the mode.** The core list rule is
  *show a setting if it's in the current mode's tier **or** it's been modified*
  — so a changed Expert setting still appears in Simple mode, across every
  category, with an `ADV`/`EXP` tier-tag marking that it's above the active
  mode. This is what lets the user always see (and revert) what differs.
- **Auto-clear:** if the user is in show-modified mode and clears their last
  override, the filter drops itself so the list isn't stuck empty.
- **Empty state:** when a mode hides everything, the panel offers a "switch to
  Expert" link.

## Implementation

- `isModified(key)` — layer-aware: `key in projectOverrides` (Project tab) or
  `key in objectOverrides` (Object tab).
- `visibleOptions` filters `options` by `(passesMode(opt, mode) ||
  isModified(opt.key))` plus printer-aware visibility + search, then narrows to
  modified-only when the toggle is on (`filterRow` takes a `modified` flag that
  bypasses the mode tier).
- Mode tiers come from libslic3r's per-option `mode` metadata via `passesMode`
  — no curated ID lists (per the no-hardcoded-classifications rule).
- `SettingRow` renders the `ADV`/`EXP` tier-tag when `outOfMode`.

## Dependencies

PR-4-4 (panel scaffold), PR-4-3 (mode filter + `passesMode`), PR-4-7 (override
tracking the modified count reuses).

## Out of scope

The dropped multi-baseline diff (above). Side-by-side rendering of two
cascades. Export-diff-as-cascade-fragment.
