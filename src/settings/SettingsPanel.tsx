// Phase 4 settings panel scaffold (PR-4-4) — the critical-path
// bottleneck. Subsequent tickets (4-5 .. 4-12) mount inside this.
//
// MVP responsibilities:
//   - Mount category nav (PR-4-3) + mode filter + search.
//   - Wire the cascade_resolve + slicer_options_for_printer loop.
//   - Render Field rows (PR-4-2) with the appropriate input
//     component per OptionTypeKind.
//   - Project / Object editing-context tabs (FR-UI-9). Object tab
//     disabled when no object is selected; auto-fall-back to
//     Project when the selected object goes away.
//   - The per-object override storage that the Object tab writes
//     into is a stub — PR-4-9 wires the real backend.
//
// Per the design reference in phase-4.md, the breadcrumb chip
// strip is the `accountability === "breadcrumb"` tweak from the
// mockup and is NOT shipped. PR-4-7 lifts the canonical "rule"
// surface (left-edge inset rule + authored-tier background tint)
// into the row CSS; PR-4-4 ships the row markup ready to carry
// those modifier classes once PR-4-7 lands.

import { useEffect, useMemo, useRef, useState } from "react";
import {
  BoolInput,
  ColorInput,
  DropdownInput,
  Field,
  MultiSelectInput,
  NumberInput,
  PercentInput,
} from "./inputs";
import { SlotTabStrip, useSlotState, type SlotInfo } from "./slots/SlotTabStrip";
import {
  CategorySidebar,
  ModeFilterControl,
  categorize,
  categoryCounts,
  passesMode,
  useStoredModeFilter,
  type ModeFilter,
} from "./nav";
import type {
  OptionSummary,
  OptionTypeKind,
  PrinterAwareOptionSummary,
} from "./types";
import { isVectorKind, optionTypeKind } from "./types";
import {
  usePrinterOptions,
  useCascadeResolve,
  type ContextJson,
  type PrinterProfileJson,
  type ResolvedMap,
} from "./resolve";
import { winningLayerFor } from "./layers";

export interface SettingsPanelProps {
  /** Active printer profile. `null` = no printer selected; the panel
   *  renders an empty state and does no resolves. */
  printer: PrinterProfileJson | null;
  /** Loaded cascade handle. `null` = no cascade loaded. */
  cascadeHandle: number | null;
  /** Cascade resolve context (printer + plate + filaments). Built
   *  by the panel's host (App.tsx) from current scene state. */
  context: ContextJson | null;
  /** Currently selected scene object — drives the Object tab.
   *  `null` disables the Object tab. */
  selectedObject: SelectedObjectStub | null;
  /** Stubbed per-object override storage. PR-4-9 replaces with the
   *  real `scene_object_override_set/clear` plumbing. */
  objectOverrides: Record<string, string>;
  onSetObjectOverride: (key: string, value: string) => void;
  onClearObjectOverride: (key: string) => void;
  /** Stubbed project-tier override map (extends `ContextJson`
   *  before resolve). Real storage lands with Phase 5's project
   *  model; PR-4-4 keeps it inline so the editing-context flow is
   *  exercisable today. */
  projectOverrides: Record<string, string>;
  onSetProjectOverride: (key: string, value: string) => void;
  onClearProjectOverride: (key: string) => void;
}

/** Minimal selected-object shape the Object tab needs. Scene state
 *  carries more; the panel only reads name + id. */
export type SelectedObjectStub = {
  id: number;
  name: string;
};

type ContextLayer = "project" | "object";

export function SettingsPanel(props: SettingsPanelProps) {
  const {
    printer,
    cascadeHandle,
    context,
    selectedObject,
    objectOverrides,
    onSetObjectOverride,
    onClearObjectOverride,
    projectOverrides,
    onSetProjectOverride,
    onClearProjectOverride,
  } = props;

  const [mode, setMode] = useStoredModeFilter();
  const [search, setSearch] = useState("");
  const [contextLayer, setContextLayer] = useState<ContextLayer>("project");
  const [activeCat, setActiveCat] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const slotCount = printer?.slot_count ?? 1;
  const { activeSlot, setActiveSlot, syncAll, setSyncAll } = useSlotState(slotCount);
  // Slot info for the tab strip. PR-4-6 ships the index-only labels;
  // PR-7c (filament sync) will populate color + label per slot
  // binding.
  const slots = useMemo<SlotInfo[]>(
    () =>
      Array.from({ length: slotCount }, (_, i) => ({
        index: i + 1,
      })),
    [slotCount],
  );

  // Object tab auto-fall-back: when the selected object disappears
  // while the Object tab is active, fall back to Project (mirrors
  // the mockup's `useSPE` at SettingsPanel.jsx:373-375).
  useEffect(() => {
    if (contextLayer === "object" && selectedObject == null) {
      setContextLayer("project");
    }
  }, [contextLayer, selectedObject]);

  const { options, loading: optsLoading } = usePrinterOptions(printer);
  const { resolved, error: resolveError } = useCascadeResolve(
    cascadeHandle,
    context,
  );

  // Apply visibility + mode + search to the option list. Pure;
  // memoize to keep the render path bounded.
  const visibleOptions = useMemo(() => {
    return options.filter((o) => filterRow(o, mode, search));
  }, [options, mode, search]);

  const groups = useMemo(() => categorize(visibleOptions), [visibleOptions]);

  const overriddenKeys = useMemo(() => {
    // Project + object overrides combined for the per-category
    // override-count badge. PR-4-7 will refine to read from the
    // cascade trace (which knows the actual winning layer).
    const out = new Set<string>();
    for (const k of Object.keys(projectOverrides)) out.add(k);
    for (const k of Object.keys(objectOverrides)) out.add(k);
    return out;
  }, [projectOverrides, objectOverrides]);

  const counts = useMemo(
    () => categoryCounts(groups, overriddenKeys),
    [groups, overriddenKeys],
  );

  const totalOverrides = overriddenKeys.size;
  const totalCategoriesWithOverrides = useMemo(
    () =>
      [...counts.values()].filter((c) => c.overrides > 0).length,
    [counts],
  );

  // Keep the active category valid as the visible list changes.
  useEffect(() => {
    if (groups.length === 0) {
      setActiveCat(null);
      return;
    }
    if (activeCat == null || !groups.some((g) => g.id === activeCat)) {
      setActiveCat(groups[0].id);
    }
  }, [groups, activeCat]);

  const jumpToCategory = (id: string) => {
    setActiveCat(id);
    const el = scrollRef.current?.querySelector<HTMLElement>(
      `[data-cat-id="${cssEscape(id)}"]`,
    );
    if (el) el.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  const objectTabAvailable = selectedObject != null;

  return (
    <section className="settings-panel" data-context-layer={contextLayer}>
      <header className="sp-header">
        <div className="sp-tabs" role="tablist" aria-label="Editing context">
          <button
            type="button"
            role="tab"
            aria-selected={contextLayer === "project"}
            className={`sp-tab${contextLayer === "project" ? " active" : ""}`}
            onClick={() => setContextLayer("project")}
          >
            Project
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={contextLayer === "object"}
            disabled={!objectTabAvailable}
            className={`sp-tab${contextLayer === "object" ? " active" : ""}`}
            onClick={() => objectTabAvailable && setContextLayer("object")}
            title={
              objectTabAvailable
                ? `Per-object overrides for ${selectedObject!.name}`
                : "Select an object on the plate to edit per-object overrides"
            }
          >
            {objectTabAvailable ? `Object: ${selectedObject!.name}` : "Object"}
          </button>
        </div>
        <ModeFilterControl
          value={mode}
          onChange={setMode}
          allowDevelop={import.meta.env.DEV}
        />
        <div className="search-wrap flex items-center gap-2">
          <input
            type="search"
            className="search-input"
            placeholder="Search 800+ settings…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
          {totalOverrides > 0 && (
            <span
              className="sp-overrides-badge px-2 py-0.5 rounded text-xs bg-amber-100 text-amber-900 dark:bg-amber-900/50 dark:text-amber-200"
              title={`${totalOverrides} settings overridden across ${totalCategoriesWithOverrides} categor${
                totalCategoriesWithOverrides === 1 ? "y" : "ies"
              }`}
            >
              {totalOverrides} overridden
            </span>
          )}
        </div>
        {slotCount >= 2 && (
          <SlotTabStrip
            slots={slots}
            activeSlot={activeSlot}
            onActiveSlotChange={setActiveSlot}
            syncAll={syncAll}
            onSyncAllChange={setSyncAll}
          />
        )}
      </header>

      {resolveError && (
        <div className="sp-error" role="alert">
          cascade resolve failed: {resolveError}
        </div>
      )}

      {printer == null ? (
        <div className="sp-empty">No printer selected.</div>
      ) : optsLoading ? (
        <div className="sp-empty">Loading options…</div>
      ) : groups.length === 0 ? (
        <div className="sp-empty">
          No matching settings — try broadening the search or mode.
        </div>
      ) : (
        <div className="settings-body">
          <CategorySidebar
            groups={groups}
            counts={counts}
            activeId={activeCat}
            onActivate={jumpToCategory}
          />
          <div className="settings-scroll" ref={scrollRef}>
            {groups.map((g) => (
              <section key={g.id} className="cat-group" data-cat-id={g.id}>
                <header className="cat-header">
                  <span className="cat-rail-icon" aria-hidden>
                    {g.icon}
                  </span>
                  <h4 className="cat-name">{g.name}</h4>
                  <span className="cat-counts">{g.settings.length}</span>
                </header>
                {g.settings.map((opt) => (
                  <SettingRow
                    key={opt.key}
                    schema={opt}
                    resolved={resolved}
                    contextLayer={contextLayer}
                    projectOverrides={projectOverrides}
                    objectOverrides={objectOverrides}
                    onSetProjectOverride={onSetProjectOverride}
                    onClearProjectOverride={onClearProjectOverride}
                    onSetObjectOverride={onSetObjectOverride}
                    onClearObjectOverride={onClearObjectOverride}
                    notApplicable={opt.hidden}
                    slotCount={slotCount}
                    activeSlot={activeSlot}
                    syncAll={syncAll}
                  />
                ))}
              </section>
            ))}
          </div>
        </div>
      )}
    </section>
  );
}

interface SettingRowProps {
  schema: OptionSummary;
  resolved: ResolvedMap;
  contextLayer: ContextLayer;
  projectOverrides: Record<string, string>;
  objectOverrides: Record<string, string>;
  onSetProjectOverride: (key: string, value: string) => void;
  onClearProjectOverride: (key: string) => void;
  onSetObjectOverride: (key: string, value: string) => void;
  onClearObjectOverride: (key: string) => void;
  /** True when this option is capability-hidden but surfaced via
   *  search; renders the "not applicable" badge inline (PR-4-5). */
  notApplicable?: boolean;
  /** Slot-adaptive layout state (PR-4-6). Vector-typed options
   *  render only the active slot's value; commits land at that
   *  index, broadcast to all when syncAll is true. */
  slotCount: number;
  activeSlot: number;
  syncAll: boolean;
}

function SettingRow({
  schema,
  resolved,
  contextLayer,
  projectOverrides,
  objectOverrides,
  onSetProjectOverride,
  onSetObjectOverride,
  notApplicable = false,
  slotCount,
  activeSlot,
  syncAll,
}: SettingRowProps) {
  const tierValue = contextLayer === "object"
    ? objectOverrides[schema.key]
    : projectOverrides[schema.key];
  const effectiveValue =
    tierValue ?? resolved[schema.key]?.value ?? schema.default_value ?? null;

  // Project-scope settings are read-only on the Object tab per
  // FR-3D-3 (the value belongs to a higher tier than per-object can
  // edit). PR-4-9 surfaces the "project-scope setting" badge; PR-4-4
  // just enforces disabled-input.
  const disabled =
    notApplicable ||
    (contextLayer === "object" &&
      schema.scope.project &&
      !schema.scope.object &&
      !schema.scope.region);

  const setValue = (next: string) => {
    if (contextLayer === "object") onSetObjectOverride(schema.key, next);
    else onSetProjectOverride(schema.key, next);
  };

  const leadingBadge = notApplicable ? (
    <span
      className="set-badge set-badge-na"
      title="Not applicable to the active printer"
    >
      not applicable
    </span>
  ) : null;

  const kind = optionTypeKind(schema);
  const winningLayer = winningLayerFor(
    schema.key,
    projectOverrides,
    objectOverrides,
  );
  return (
    <Field
      schema={schema}
      value={effectiveValue}
      onChange={setValue}
      disabled={disabled}
      leadingBadge={leadingBadge}
      winningLayer={winningLayer}
    >
      {isVectorKind(kind) && slotCount >= 1 ? (
        <MultiSelectInput
          schema={schema}
          value={effectiveValue}
          onChange={setValue}
          disabled={disabled}
          slotCount={slotCount}
          activeSlot={activeSlot}
          syncAll={syncAll}
          renderSlot={({ value, onChange: onSlotChange, disabled: slotDisabled }) =>
            renderScalarInput(
              vectorElementKind(kind),
              schema,
              value,
              onSlotChange,
              slotDisabled,
            )
          }
        />
      ) : (
        renderScalarInput(kind, schema, effectiveValue, setValue, disabled)
      )}
    </Field>
  );
}

/** Map the vector kind to the scalar inner kind so the renderSlot
 *  callback can route through the same scalar renderer. */
function vectorElementKind(kind: OptionTypeKind): OptionTypeKind {
  switch (kind) {
    case "vector-bool":
      return "bool";
    case "vector-int":
      return "int";
    case "vector-float":
      return "float";
    case "vector-percent":
      return "percent";
    case "vector-float-or-percent":
      return "float-or-percent";
    case "vector-string":
      return "string";
    case "vector-enum":
      return "enum";
    default:
      return "unknown";
  }
}

/** Render the per-slot scalar input. Vector kinds are unwrapped by
 *  the SettingRow's MultiSelectInput layer; this function only sees
 *  scalar kinds (single-slot value at a time). */
function renderScalarInput(
  kind: OptionTypeKind,
  schema: OptionSummary,
  value: string | null,
  onChange: (next: string) => void,
  disabled: boolean,
) {
  switch (kind) {
    case "bool":
      return (
        <BoolInput
          schema={schema}
          value={value}
          onChange={onChange}
          disabled={disabled}
        />
      );
    case "float":
    case "int":
      return (
        <NumberInput
          schema={schema}
          value={value}
          onChange={onChange}
          disabled={disabled}
        />
      );
    case "percent":
    case "float-or-percent":
      return (
        <PercentInput
          schema={schema}
          value={value}
          onChange={onChange}
          disabled={disabled}
        />
      );
    case "color":
      return (
        <ColorInput
          schema={schema}
          value={value}
          onChange={onChange}
          disabled={disabled}
        />
      );
    case "enum":
      // Until PR-4-1's OptionSummary surfaces enum_values, we
      // render a plain text input for enums. The dropdown is
      // ready to mount when the schema gains the field.
      return (
        <DropdownInput
          schema={schema}
          value={value}
          onChange={onChange}
          disabled={disabled}
          options={[]}
        />
      );
    case "string":
    case "point":
    case "point3":
    case "unknown":
    default:
      // Fallback to a plain text input for scalar kinds the form
      // library doesn't yet specialize for. Vector kinds are
      // handled in SettingRow above and never reach here.
      return (
        <input
          className="val-input val-input-fallback"
          type="text"
          value={value ?? ""}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
        />
      );
  }
}

/** Pure filter function used by the panel and exposed for vitest. */
export function filterRow(
  opt: PrinterAwareOptionSummary,
  mode: ModeFilter,
  search: string,
): boolean {
  if (opt.hidden) {
    // Match the mockup behavior: when search is active, hidden
    // options are shown with a "not applicable" badge. PR-4-5
    // controls the rendering; PR-4-4's filter excludes them in
    // the no-search default view.
    if (search.trim() === "") return false;
  }
  if (!passesMode(opt, mode)) return false;
  if (search.trim() === "") return true;
  const needle = search.toLowerCase();
  return (
    opt.key.toLowerCase().includes(needle) ||
    (opt.label?.toLowerCase().includes(needle) ?? false) ||
    (opt.category?.toLowerCase().includes(needle) ?? false)
  );
}

/** Minimal CSS.escape polyfill — Tauri's bundled CSS environment has
 *  the global, but we don't rely on it for cross-runtime safety. */
function cssEscape(s: string): string {
  return s.replace(/[^a-zA-Z0-9_-]/g, (c) => `\\${c}`);
}
