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
  NumberInput,
  PercentInput,
} from "./inputs";
import {
  CategorySidebar,
  categorize,
  categoryCounts,
  passesMode,
  type ModeFilter,
} from "./nav";
import type {
  OptionSummary,
  OptionTypeKind,
  PrinterAwareOptionSummary,
} from "./types";
import {
  defaultMultilineText,
  defaultScalarFor,
  isMultilineTextField,
  isObjectOverridable,
  isVectorKind,
  optionTypeKind,
} from "./types";
import {
  usePrinterOptions,
  type PrinterProfileJson,
  type ResolvedMap,
} from "./resolve";
import { winningLayerFor, type CascadeLayer } from "./layers";
import { CascadeLadder, useLadderHover } from "./ladder/CascadeLadder";
import { ANNOTATIONS } from "./annotations/data";
import {
  computeDiff,
  passesDiff,
  readStoredDiffMode,
  writeStoredDiffMode,
  type DiffMode,
} from "./diff";
import { PluginManager, type PluginWriters } from "../plugins/PluginManager";
import {
  countActiveAtLevel,
  type CascadeSources,
} from "../plugins/pluginCascade";
import type { PluginSummary } from "../plugins/pluginCommands";

/** The plate-level plugin surface, rendered in the right-aligned
 *  Plugins tab. Built by the panel host from the active plate. */
export interface PluginPlateSurface {
  plugins: PluginSummary[];
  sources: CascadeSources;
  writers: PluginWriters;
  plateName: string | null;
}

export interface SettingsPanelProps {
  /** Active printer profile. `null` = no printer selected; the panel
   *  renders an empty state and does no resolves. */
  printer: PrinterProfileJson | null;
  /** The active plate's cascade resolution (from `plate_cascade_resolve`),
   *  keyed by setting. Owned + fetched by the host (it has the plate
   *  state to key on); the panel reads values + their `source_layer` to
   *  fill the cascade rows. Empty `{}` when there's no resolved plate. */
  resolved: ResolvedMap;
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
  /** All objects on the plate, each with its own override map.
   *  Used to render the FR-CAS-7b "N objects override" badge on
   *  Project-tab rows. Empty by default — PR-5's project model
   *  populates. */
  allObjects?: ReadonlyArray<PlateObjectStub>;
  /** Plate-level plugin surface (the right-aligned Plugins tab). When
   *  omitted the tab isn't shown. */
  pluginSurface?: PluginPlateSurface | null;
}

/** Per-object override info for the objects-overriding badge. */
export type PlateObjectStub = {
  id: number;
  name: string;
  /** Filament color for the badge swatch. */
  color?: string | null;
  overrides: Record<string, string>;
};

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
    resolved,
    selectedObject,
    objectOverrides,
    onSetObjectOverride,
    onClearObjectOverride,
    projectOverrides,
    onSetProjectOverride,
    onClearProjectOverride,
    allObjects = [],
    pluginSurface = null,
  } = props;

  // The right-aligned Plugins tab swaps the whole settings body for the
  // plate-level plugin manager. Independent of the project/object
  // editing-context tabs. `onPluginTab` also guards the surface
  // vanishing (no active plate) — the latch can stay set, but the tab is
  // only "on" while the surface exists, so a normal tab stays highlighted.
  const [pluginTabActive, setPluginTabActive] = useState(false);
  const onPluginTab = pluginTabActive && pluginSurface != null;

  // Mode filter UI is parked pending a redesign; pin to "advanced"
  // — "expert" pulls in G-code / machine-limits noise most users
  // never touch. `setMode` is kept available for when the new UI
  // lands.
  const [mode, setMode] = useState<ModeFilter>("advanced");
  void setMode;
  const [search, setSearch] = useState("");
  const [contextLayer, setContextLayer] = useState<ContextLayer>("project");
  const [activeCat, setActiveCat] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // Object tab auto-fall-back: when the selected object disappears
  // while the Object tab is active, fall back to Project (mirrors
  // the mockup's `useSPE` at SettingsPanel.jsx:373-375).
  useEffect(() => {
    if (contextLayer === "object" && selectedObject == null) {
      setContextLayer("project");
    }
  }, [contextLayer, selectedObject]);

  const { options, loading: optsLoading } = usePrinterOptions(printer);

  // Diff baselines (PR-4-10):
  //   - "from-default": cascade resolved with no overrides. We
  //     approximate by tracking the resolved map sans overrides
  //     for the same printer + plate + filaments. Since the panel
  //     receives `context` with overrides already merged, we
  //     compute a printer-only baseline by stripping overrides
  //     and re-resolving once per printer change. (Phase 5's
  //     project model adds dedicated baseline tracking.)
  //   - "from-save": snapshot of `resolved` at panel mount or at
  //     project-save time. In-memory only for Phase 4; Phase 5's
  //     project save populates from the .3mf load.
  // Diff tabs (All / Diff: default / Diff: save) are parked pending
  // a redesign; pin to "all" so every row shows. `setDiffMode` is
  // kept available for when the new UI lands.
  const [diffMode, setDiffMode] = useState<DiffMode>("all");
  void setDiffMode;
  void readStoredDiffMode;
  void writeStoredDiffMode;
  const savedBaselineRef = useRef<ResolvedMap | null>(null);
  useEffect(() => {
    if (savedBaselineRef.current === null && Object.keys(resolved).length > 0) {
      savedBaselineRef.current = { ...resolved };
    }
  }, [resolved]);
  // For "from-default" we approximate with the cascade resolve
  // minus project + object overrides — those are the tiers we
  // know about at the frontend. Phase 5's project model adds a
  // separate printer-only resolve.
  const defaultBaseline = useMemo<ResolvedMap>(() => {
    const out: ResolvedMap = {};
    for (const [k, v] of Object.entries(resolved)) {
      if (k in projectOverrides || k in objectOverrides) {
        out[k] = { ...v, value: v.cascade_fallback ?? v.value };
      } else {
        out[k] = v;
      }
    }
    return out;
  }, [resolved, projectOverrides, objectOverrides]);
  const diffSet = useMemo(() => {
    if (diffMode === "all") return new Set<string>();
    if (diffMode === "from-default") return computeDiff(resolved, defaultBaseline);
    return computeDiff(resolved, savedBaselineRef.current ?? resolved);
  }, [diffMode, resolved, defaultBaseline]);

  // Apply visibility + mode + search + diff to the option list.
  // Pure; memoize to keep the render path bounded.
  const visibleOptions = useMemo(() => {
    return options.filter(
      (o) => filterRow(o, mode, search) && passesDiff(o.key, diffMode, diffSet),
    );
  }, [options, mode, search, diffMode, diffSet]);

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

  // Cascade ladder hover state (PR-4-8). One ladder portal per
  // panel; tracks which row triggered it so the per-row data the
  // ladder reads stays addressable on hover.
  const ladder = useLadderHover();
  const [hoveredKey, setHoveredKey] = useState<string | null>(null);
  const hoveredSchema = useMemo(
    () => visibleOptions.find((o) => o.key === hoveredKey) ?? null,
    [visibleOptions, hoveredKey],
  );

  // PR-4-11's separate `SettingTooltip` popover was folded into the
  // cascade ladder so a single row hover surfaces both the
  // description and the layer breakdown.

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
            aria-selected={!onPluginTab && contextLayer === "project"}
            className={`sp-tab${!onPluginTab && contextLayer === "project" ? " active" : ""}`}
            onClick={() => {
              setPluginTabActive(false);
              setContextLayer("project");
            }}
          >
            Project
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={!onPluginTab && contextLayer === "object"}
            disabled={!objectTabAvailable}
            className={`sp-tab${!onPluginTab && contextLayer === "object" ? " active" : ""}`}
            onClick={() =>
              objectTabAvailable &&
              (setPluginTabActive(false), setContextLayer("object"))
            }
            title={
              objectTabAvailable
                ? `Per-object overrides for ${selectedObject!.name}`
                : "Select an object on the plate to edit per-object overrides"
            }
          >
            {objectTabAvailable ? `Object: ${selectedObject!.name}` : "Object"}
          </button>
          {pluginSurface && (
            <>
              <div className="sp-tabs-spacer" />
              <button
                type="button"
                role="tab"
                aria-selected={onPluginTab}
                className={`sp-tab${onPluginTab ? " active" : ""}`}
                style={{ "--tab-hue": 340 } as React.CSSProperties}
                onClick={() => setPluginTabActive(true)}
                title="Plugins enabled for this plate (overrides Global and Project)"
              >
                <span className="sp-tab-dot" />
                Plugins
                {(() => {
                  const n = countActiveAtLevel(
                    pluginSurface.plugins,
                    "plate",
                    pluginSurface.sources,
                  );
                  return n > 0 ? <span className="sp-tab-count">{n}</span> : null;
                })()}
              </button>
            </>
          )}
        </div>
        {/* Mode filter (Simple / Advanced / Expert / Develop) and the
            diff tabs are intentionally not rendered yet; the user is
            redesigning that surface. The underlying state + filter
            machinery is still in place (mode is pinned to "expert" so
            all stable settings show; diffMode stays at "all").
            The per-extruder slot strip + sync-edit toggle retired
            with PR-S-2's Process-only filter — there are no per-
            extruder options surfaced here for a slot picker to act
            on. Filament/printer-bucket editing surfaces live
            elsewhere. */}
        <div
          className="search-wrap"
          style={onPluginTab ? { display: "none" } : undefined}
        >
          <div className="search-input">
            <input
              type="search"
              placeholder="Search 800+ settings…"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
            {totalOverrides > 0 && (
              <span
                className="sp-overrides-badge"
                title={`${totalOverrides} settings overridden across ${totalCategoriesWithOverrides} categor${
                  totalCategoriesWithOverrides === 1 ? "y" : "ies"
                }`}
              >
                {totalOverrides} overridden
              </span>
            )}
          </div>
        </div>
      </header>

      {onPluginTab && pluginSurface ? (
        <div className="sp-plugins-scroll">
          <PluginManager
            level="plate"
            plugins={pluginSurface.plugins}
            sources={pluginSurface.sources}
            writers={pluginSurface.writers}
            plateName={pluginSurface.plateName}
          />
        </div>
      ) : (
        <>
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
                    allObjects={allObjects}
                    onRowEnter={(el) => {
                      setHoveredKey(opt.key);
                      ladder.openLadder(el);
                    }}
                    onRowLeave={ladder.scheduleClose}
                  />
                ))}
              </section>
            ))}
          </div>
        </div>
      )}
      {hoveredSchema && (
        <CascadeLadder
          settingKey={hoveredSchema.key}
          settingLabel={hoveredSchema.label ?? hoveredSchema.key}
          layers={buildLadderLayers(hoveredSchema, resolved, projectOverrides, objectOverrides)}
          winningLayer={ladderWinningLayer(
            hoveredSchema.key,
            resolved,
            projectOverrides,
            objectOverrides,
          )}
          anchor={ladder.anchor}
          open={ladder.open}
          onMouseEnter={() => ladder.openLadder(ladder.anchor!)}
          onMouseLeave={ladder.scheduleClose}
          cascadeFallback={resolved[hoveredSchema.key]?.cascade_fallback ?? null}
          description={hoveredSchema.tooltip ?? null}
          whyThisMatters={ANNOTATIONS[hoveredSchema.key] ?? null}
          objectOverrides={
            contextLayer === "project"
              ? allObjects
                  .filter((o) => hoveredSchema.key in o.overrides)
                  .map((o) => ({
                    id: o.id,
                    name: o.name,
                    color: o.color,
                    value: o.overrides[hoveredSchema.key],
                  }))
              : []
          }
        />
      )}
        </>
      )}
    </section>
  );
}

/** The ladder's winner ✓: an override tier when one wins, else the
 *  cascade layer the resolved value was attributed to (so the ✓ lands on
 *  e.g. "Profile" when the process fragment is the winner). Distinct from
 *  `winningLayerFor`, which stays "cascade" for the row-tint logic that
 *  must not treat a fragment win as user-authored. */
function ladderWinningLayer(
  key: string,
  resolved: ResolvedMap,
  projectOverrides: Record<string, string>,
  objectOverrides: Record<string, string>,
): CascadeLayer {
  const base = winningLayerFor(key, projectOverrides, objectOverrides);
  if (base !== "cascade") return base;
  return (resolved[key]?.source_layer as CascadeLayer | undefined) ?? "cascade";
}

/** Build the per-layer value snapshot the CascadeLadder reads.
 *  `default` is the engine schema default. The cascade-resolved value
 *  (`plate_cascade_resolve`) is placed under the layer it won from —
 *  its `source_layer` — so e.g. a process-fragment value shows under
 *  "Profile" (the `user` row) and a bed-fragment value under "Build
 *  plate". The override tiers (`project` / `object`) come straight from
 *  the panel's own override maps. Cascade rows with no contribution
 *  render as em-dash. */
function buildLadderLayers(
  schema: OptionSummary,
  resolved: ResolvedMap,
  projectOverrides: Record<string, string>,
  objectOverrides: Record<string, string>,
): Map<CascadeLayer, string | null> {
  const map = new Map<CascadeLayer, string | null>();
  // `default` is the engine schema default — the genuine bottom of the
  // ladder, distinct from what our fragments resolve to.
  map.set("default", defaultScalarFor(schema));
  map.set("printer", null);
  map.set("build_plate", null);
  map.set("nozzle", null);
  map.set("filament", null);
  map.set("user", null);
  // Place the cascade-resolved value under the cascade layer it won
  // from. `source_layer` is a CascadeLayer id from the backend
  // (process → "user"/Profile, nozzle → "nozzle", bed → "build_plate",
  // filament → "filament", machine/topology → "printer"); fall back to
  // "printer" so a value is never silently dropped.
  const entry = resolved[schema.key];
  if (entry != null) {
    const layer = (entry.source_layer ?? "printer") as CascadeLayer;
    if (map.has(layer)) map.set(layer, entry.value);
  }
  map.set("project", projectOverrides[schema.key] ?? null);
  map.set("object", objectOverrides[schema.key] ?? null);
  return map;
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
  /** Cascade ladder hover hooks (PR-4-8). The panel owns the
   *  open/close lifecycle centrally; SettingRow just forwards the
   *  row's DOM node + leave. The label hover hooks retired with
   *  the SettingTooltip merge — description lives in the ladder. */
  onRowEnter?: (el: HTMLElement) => void;
  onRowLeave?: () => void;
  /** All objects on the plate (PR-4-9) — drives the objects-
   *  overriding badge on Project-tab rows. Empty by default. */
  allObjects: ReadonlyArray<PlateObjectStub>;
}

function SettingRow({
  schema,
  resolved,
  contextLayer,
  projectOverrides,
  objectOverrides,
  onSetProjectOverride,
  onClearProjectOverride,
  onSetObjectOverride,
  onClearObjectOverride,
  notApplicable = false,
  onRowEnter,
  onRowLeave,
  allObjects,
}: SettingRowProps) {
  const tierValue = contextLayer === "object"
    ? objectOverrides[schema.key]
    : projectOverrides[schema.key];
  // Multiline coStrings (`start_gcode`, the small-area infill flow
  // compensation model, …) carry one entry per line of a single
  // logical text block. The `\n`-joined textarea view is what
  // displays; per-slot indexing is meaningless here.
  const fallbackDefault = isMultilineTextField(schema)
    ? defaultMultilineText(schema)
    : defaultScalarFor(schema);
  const effectiveValue =
    tierValue ?? resolved[schema.key]?.value ?? fallbackDefault ?? null;

  // On the Object tab only object/region-scoped settings are editable —
  // they're the only ones libslic3r honors per object, mirroring the
  // slice-time gate (`object_overrides_for_slice`). Anything else (a
  // project/print-scope setting, or a dangling no-scope option like
  // `ironing_expansion`) is disabled, so the user can't author an
  // override the slicer would silently drop. PR-4-9 surfaces the
  // "project-scope setting" badge; this just enforces disabled-input.
  const disabled =
    notApplicable ||
    (contextLayer === "object" && !isObjectOverridable(schema.scope));

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

  // Objects-overriding badge (PR-4-9, FR-CAS-7b): on the Project
  // tab, surface the objects that override this setting via small
  // filament-color dots + a count.
  const overridingObjects =
    contextLayer === "project"
      ? allObjects.filter((o) => schema.key in o.overrides)
      : [];
  const trailingBadge = overridingObjects.length > 0 ? (
    <span
      className="objs-badge"
      title={`${overridingObjects.length} object${
        overridingObjects.length === 1 ? "" : "s"
      } override this setting`}
    >
      {overridingObjects.slice(0, 3).map((o) => (
        <span
          key={o.id}
          className="objs-badge-dot"
          style={{ background: o.color ?? "#888" }}
          aria-hidden
        />
      ))}
      {overridingObjects.length > 3 && (
        <span className="objs-badge-more">+{overridingObjects.length - 3}</span>
      )}
    </span>
  ) : null;

  // Reset button (PR-4-9). Renders when the active tier has a
  // value for this setting; clicking drops the override and the
  // row falls back to the cascade resolution underneath.
  const hasValueAtActiveTier =
    contextLayer === "object"
      ? schema.key in objectOverrides
      : schema.key in projectOverrides;
  const resetButton = hasValueAtActiveTier ? (
    <button
      type="button"
      className="reset-btn"
      title={`Reset ${contextLayer} override (falls back to inherited value)`}
      onClick={(e) => {
        e.stopPropagation();
        if (contextLayer === "object") onClearObjectOverride(schema.key);
        else onClearProjectOverride(schema.key);
      }}
      aria-label={`Reset ${schema.key} ${contextLayer} override`}
    >
      <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
        <path
          d="M2.5 5a3.5 3.5 0 1 0 1-2.5"
          stroke="currentColor"
          strokeWidth="1.4"
          strokeLinecap="round"
        />
        <path
          d="M2 2v3h3"
          stroke="currentColor"
          strokeWidth="1.4"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </button>
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
      trailingBadge={trailingBadge}
      resetButton={resetButton}
      winningLayer={winningLayer}
      onRowEnter={onRowEnter}
      onRowLeave={onRowLeave}
    >
      {isMultilineTextField(schema) ? (
        // Multiline coStrings → single textarea showing all entries
        // joined with `\n`. Read-only for now — a real textarea
        // Field with cstyle-aware commit ships later.
        <textarea
          className="val-input val-input-multiline"
          value={effectiveValue ?? ""}
          disabled
          readOnly
          rows={Math.min(
            10,
            Math.max(2, (effectiveValue?.split("\n").length ?? 1)),
          )}
        />
      ) : isVectorKind(kind) ? (
        // Other vector kinds (`vector-int`, `vector-float`, etc.)
        // surface in the Process bucket through dimensional families
        // (e.g. bed_temp expands per plate type) — but the panel
        // doesn't have an editor for those yet. Show read-only text
        // so the user can see the value without a path to corrupt
        // it. Per-extruder editing surfaces (filament/printer
        // buckets) live elsewhere; the slot picker that used to
        // mount here retired with PR-S-2's Process-only filter.
        <input
          className="val-input val-input-fallback"
          type="text"
          value={effectiveValue ?? ""}
          readOnly
          disabled
        />
      ) : (
        renderScalarInput(kind, schema, effectiveValue, setValue, disabled)
      )}
    </Field>
  );
}

/** Render a scalar input for a single-value option. */
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
      return (
        <DropdownInput
          schema={schema}
          value={value}
          onChange={onChange}
          disabled={disabled}
          options={schema.enum_values}
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
