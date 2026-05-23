# PR-4-3 — Category navigation + mode filter

Status: ❌ open.

**Scope.** The category sidebar that groups options into navigable
buckets (Quality, Strength, Speed, Travel, Multiple Extruders,
Support, Adhesion, etc.) plus the Simple / Advanced / Expert mode
filter that's a horizontal toggle at the top of the panel.

Owns FR-UI-1 (category grouping) and FR-UI-2 (mode filter). Lives
in `src/settings/nav/`.

**Acceptance criteria.**

- New module `src/settings/nav/categories.ts`:
  - `function categorize(options: OptionSummary[]):
     Map<string, OptionSummary[]>` — groups options by their
    `category` field. Preserves libslic3r's declaration order
    within each category (the order PR-4-1's `slicer_options`
    returns).
  - Category order is libslic3r's canonical order from
    `PrintConfig.cpp` (Quality, Strength, Speed, …), not
    alphabetical. Hardcode the order list with a fallback for
    unknown categories ("Other" at end).
  - Options with `category == None` go in an "Uncategorized"
    bucket at the end (will mostly be machine-only options that
    the slot-adaptive layout in PR-4-6 handles separately).

- New component `src/settings/nav/CategorySidebar.tsx`:
  - Vertical list of categories with names + an indicator dot
    showing override count (badge slot reserved for PR-4-7).
  - Active category highlighted; clicking a category scrolls the
    settings list to its first row.
  - Keyboard navigation (up/down arrows, Enter to focus first
    setting).

- New component `src/settings/nav/ModeFilter.tsx`:
  - Horizontal segmented control: Simple / Advanced / Expert.
    Optional Develop tab gated by `import.meta.env.DEV` only.
  - Filters the visible option list by `mode` field from PR-4-1.
    Simple shows Simple-mode options; Advanced shows Simple +
    Advanced; Expert shows everything except Develop. Develop
    shows everything.
  - Default to Simple on first mount; persist the user's last
    selection in localStorage keyed by `n3o.settings.mode`.

- `src/settings/nav/index.ts` exports both components + the
  `categorize` function.

- vitest:
  - `categorize` returns a Map with libslic3r's canonical order
    and within-category preservation of declaration order.
  - Mode filter `Simple` returns only Simple options; `Advanced`
    includes Simple + Advanced; etc. (pure function over a
    fixture options array).
  - Hidden categories (zero visible options after filter) are
    elided from the sidebar.

**Effort.** ~1.5 days. Mechanical UI work; the only judgement call
is which categories to hardcode in the canonical-order list (audit
the OrcaSlicer GUI's category order to mirror it).

**Dependencies.** PR-4-1 (consumes `mode` + `category` fields).

**Out of scope.** Search-as-you-type across categories — that's
FR-UI-3, deferred to PR-4-4 (it's a panel-scaffold-level feature
since search needs to render results in the main pane). Per-category
override-count badges visualize what's there but the count source
(per-category override accumulator) lands with PR-4-7.

**Cut candidate.** Keyboard navigation in the sidebar (~half day).
Mouse-only is acceptable for the MVP; keyboard shortcut work folds
into Phase 9.

**Design reference.** Mockup's `cat-rail` (left column of
`SettingsPanel.jsx`'s `.settings-body`) is the visual target.
Each `cat-rail-item` carries an icon (single letter in the
mockup), name, and `cat-rail-count` (renders `overrides/total`
in accent color when overrides > 0, plain total otherwise). The
production component should use the same class names and the
same "X/Y" badge shape. Category icons: the mockup uses
hand-picked single letters (Q, S, W, …) per category — for the
production version, derive from libslic3r's category names with
a small hardcoded letter-or-glyph map (TBD during
implementation; pick what reads at 12px).

The mode filter doesn't have a direct mockup counterpart (the
mockup shows all settings regardless of mode); follow the
"segmented control" visual idiom used elsewhere in the mockup
(`.sp-tabs`, the Project/Object tab strip — same pill shape with
underline-on-active).
