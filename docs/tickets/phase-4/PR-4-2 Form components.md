# PR-4-2 — Data-driven form component library

Status: ❌ open.

**Scope.** Six input components the settings panel renders per row,
one per libslic3r option type. Pure TS / React; no Tauri integration
and no cascade-awareness yet — the components take a typed value +
metadata and emit changes. PR-4-4's panel scaffold wires them into
the resolve-and-write flow.

Owns the **UI prerequisites** every subsequent ticket consumes.

**Acceptance criteria.**

- New `src/settings/inputs/` directory with one file per component:
  - **`NumberInput.tsx`** — for `OptType::Float` / `Int`. Renders
    a `<input type="number">` with optional unit suffix (sidetext
    from libslic3r's def), min/max enforcement, and on-blur
    commit. Sub-second debouncing for typing UX; commits on Enter
    or blur, not on every keystroke.
  - **`PercentInput.tsx`** — for `OptType::Percent` / `FloatOrPercent`.
    Number + `%` suffix; for `FloatOrPercent`, a toggle between
    "absolute" and "% of nozzle diameter" with libslic3r's exact
    canonical text ("mm" / "%").
  - **`DropdownInput.tsx`** — for `OptType::Enum`. Lists the
    schema's `enum_values` (key → label pairs); commits the
    selected key. Search-as-you-type for enums with > 8 options.
  - **`ColorInput.tsx`** — for color-typed strings (filament_colour
    et al). Swatch + hex input; falls back to plain text input
    when the value isn't a `#RRGGBB` string.
  - **`MultiSelectInput.tsx`** — for vector options
    (`OptType::Strings` / `Bools` / `Enums` per `is_vector` flag).
    Renders each entry inline with add/remove controls; vector
    length is bounded by the schema's "one entry per filament" /
    "one entry per extruder" convention which the panel scaffold
    derives from the active printer.
  - **`BoolInput.tsx`** — for `OptType::Bool`. A simple
    toggle/checkbox; commits on click.

- Common contract enforced by a shared `<Field />` wrapper:
  ```ts
  interface FieldProps {
    schema: OptionSummary;         // from PR-4-1
    value: string | null;          // serialized libslic3r value
    onChange: (next: string) => void;
    disabled?: boolean;
    error?: string;                // PR-4-11's validation surfaces here
  }
  ```
  Each input subscribes to the same shape. The wrapper renders the
  label, the input, and any error/badge slots (override badge,
  source breadcrumb) — those slots are filled by PR-4-7 + PR-4-9
  but the wrapper reserves the layout now so later tickets only
  add content.

- Storybook-style demo route (`src/settings/__demo__/InputsDemo.tsx`)
  that mounts one of each component on a stub. Not shipped in the
  production bundle (gated by `import.meta.env.DEV`). The exit
  smoke (PR-4-13) doesn't depend on this, but it makes PR-4-2's
  acceptance verifiable without booting the full panel scaffold.

- vitest unit tests, one per component, covering:
  - Renders the schema's `default_value` when `value === null`.
  - Commits the typed value (number for NumberInput, parsed key
    for DropdownInput, …) via `onChange`.
  - Honors `disabled` (no commits fire while disabled).
  - Error prop renders (shape only — visual is a smoke check).

**Effort.** ~3 days. Six components × ~3 hours each plus the shared
wrapper + tests + the demo route. The longest pole is MultiSelectInput
because vector size needs to track the active printer's slot count
(printer-dependent) and that wiring is what PR-4-4 will exercise.

**Dependencies.** PR-4-1 (consumes `OptionSummary` extensions).

**Out of scope.** Tooltips (PR-4-11). Source-layer breadcrumb (PR-4-7).
Object-overrides badge (PR-4-9). Validation error rendering with
libslic3r `config_validate` integration (PR-4-11 — this ticket only
reserves the `error` prop slot). Any cross-field gating (e.g. "show
field X iff field Y is set") — Phase 4 doesn't tackle inter-field
predicates.

**Cut candidate.** The Storybook-style demo route (~half a day).
Tests cover the per-component contract; the demo only helps manual
spot-checks.

**Design reference.** `docs/design/SettingsPanel.jsx`'s
`SettingRow::renderControl()` shows the three control flavors the
mockup uses today: `val-toggle` (boolean), `val-select`
(dropdown), and `val-input` (number with optional `val-unit`
suffix). Mirror those class names so `docs/design/styles.css`
applies. PR-4-2 expands beyond the mockup with `PercentInput`,
`ColorInput`, and `MultiSelectInput` — none of which the mockup
exercises because its hand-picked `ALL_SETTINGS` fixture happens
to be number/toggle/select only. For those three new controls,
keep the same `.val-*` outer wrapper conventions so future
styling stays uniform.
