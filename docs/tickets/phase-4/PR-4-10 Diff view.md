# PR-4-10 — Diff view (vs printer default, vs last save)

Status: ❌ open.

**Scope.** Two diff modes the user can toggle on the settings panel
to see at-a-glance what's changed:

1. **vs printer default** — show only settings whose resolved value
   differs from what the printer profile alone would produce
   (i.e. settings touched by the filament / plate / overrides).
2. **vs last save** — show only settings whose value has changed
   since the last project save. Requires Phase 5's project save
   to exist; in Phase 4 this falls back to "vs initial load" of
   the current cascade snapshot.

**Acceptance criteria.**

- New `src/settings/diff/DiffModeFilter.tsx`:
  - Pill toggle near the panel header: `All` / `Diff from default`
    / `Diff from save`.
  - Default state `All`. User selection persists in localStorage.

- New `src/settings/diff/computeDiff.ts`:
  - `function diffFromDefault(resolved: ResolvedCascade,
     printerOnly: ResolvedCascade): Set<string>` — returns the
    set of option keys whose values differ. Pure / cheap.
  - `function diffFromSave(resolved: ResolvedCascade,
     baseline: ResolvedCascade): Set<string>` — same shape.

- Backend integration:
  - For "vs printer default": call `cascade_resolve(handle,
     {printer, no filament, default plate, no overrides})` once
    per printer change → cache as the printer-only baseline.
    Diff is a key-by-key comparison against the live resolve.
  - For "vs last save": the baseline is whatever `resolved` was
    at last save time. Phase 4 stores it in memory only (no
    project save → reload yet); on settings panel mount the
    baseline becomes the current resolve.

- Panel integration:
  - Diff mode active → only rows in the diff set are rendered.
    Category sidebar shows per-category counts of diff'd rows
    (extends PR-4-7's badge mechanism).
  - Empty diff (everything matches baseline) → friendly empty
    state "No differences from {default,save}" with a link back
    to `All`.

- Smoke check:
  - On a fresh A1 mini load with no overrides, "Diff from default"
    yields the empty state.
  - Override `layer_height` at the project tier → diff mode
    shows only that row.
  - Save (in Phase 4, "snapshot baseline") → "Diff from save"
    yields empty. Edit anything → "Diff from save" shows the
    edited row.

- vitest:
  - `diffFromDefault` returns the correct key set on a fixture
    pair (5 differences expected, 5 keys returned).
  - DiffModeFilter persists selection across remount via the
    localStorage mock.

**Effort.** ~2 days. The pure-function diff is small; the bulk is
plumbing the printer-only baseline + the in-memory snapshot.

**Dependencies.** PR-4-4 (panel scaffold), PR-4-7 (category
badge mechanism for diff counts).

**Out of scope.** Diff vs another plate (cut per Execution Plan §6
— saves 2 days; requires multi-plate which is Phase 5). Side-by-
side rendering of two cascades (just the filtered list is enough
for MVP). Export-diff-as-cascade-fragment (Phase 9 polish).

**Cut candidate.** Whole ticket per Execution Plan §6 — drops the
diff modes entirely (saves 2 days). Settings still navigable;
override visibility remains via PR-4-7's badges + PR-4-8's ladder.
If cut, the smoke (PR-4-13) loses the diff verification step.
