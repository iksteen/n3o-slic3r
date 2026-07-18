// Settings panel — the cascade-resolved settings surface.
//
// Responsibilities:
//   - Mount category nav + mode filter + search.
//   - Wire the cascade_resolve + slicer_options_for_printer loop.
//   - Render Field rows with the appropriate input
//     component per OptionTypeKind.
//   - Project / Object editing-context tabs (FR-UI-9). Object tab
//     disabled when no object is selected; auto-fall-back to
//     Project when the selected object goes away.
//   - Per-object override storage backed by the scene-object override
//     backend (writes routed through the host's callbacks).
//
// Per the design reference in phase-4.md, the breadcrumb chip
// strip is the `accountability === "breadcrumb"` tweak from the
// mockup and is NOT shipped. The canonical "rule" surface (left-edge
// inset rule + authored-tier background tint) lives in the row CSS.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  CategorySidebar,
  categorize,
  categoryCounts,
  passesMode,
} from "./nav";
import {
  usePrinterOptions,
  type PrinterProfileJson,
  type ResolvedMap,
} from "./resolve";
import { CascadeLadder, useLadderHover } from "./ladder/CascadeLadder";
import { ANNOTATIONS } from "./annotations/data";
import { useStoredModeFilter } from "./nav/ModeFilter";
import { PluginManager, type PluginWriters } from "../plugins/PluginManager";
import {
  countActiveAtLevel,
  type CascadeSources,
} from "../plugins/pluginCascade";
import type { PluginSummary } from "../plugins/pluginCommands";
import { SettingRow } from "./SettingRow";
import {
  buildLadderLayers,
  filterRow,
  ladderWinningLayer,
  type ContextLayer,
} from "./settingsPanelHelpers";

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
  selectedObject: SelectedObject | null;
  /** Per-object override storage, backed by the `scene_object_override`
   *  set/clear plumbing through the host's callbacks below. */
  objectOverrides: Record<string, string>;
  onSetObjectOverride: (key: string, value: string) => void;
  onClearObjectOverride: (key: string) => void;
  /** Project-tier override map (applied as a cascade override tier
   *  before resolve), backed by the project model's stored overrides. */
  projectOverrides: Record<string, string>;
  onSetProjectOverride: (key: string, value: string) => void;
  onClearProjectOverride: (key: string) => void;
  /** All objects on the plate, each with its own override map.
   *  Used to render the FR-CAS-7b "N objects override" badge on
   *  Project-tab rows. */
  allObjects?: ReadonlyArray<PlateObject>;
  /** Plate-level plugin surface (the right-aligned Plugins tab). When
   *  omitted the tab isn't shown. */
  pluginSurface?: PluginPlateSurface | null;
}

/** Per-object override info for the objects-overriding badge. */
export type PlateObject = {
  id: number;
  name: string;
  /** Filament color for the badge swatch. */
  color?: string | null;
  overrides: Record<string, string>;
};

/** Minimal selected-object shape the Object tab needs. Scene state
 *  carries more; the panel only reads name + id + kind. `kind` is
 *  "group" when the selection is a whole group — the tab reads
 *  "Group: name" because object-scope edits apply to every member. */
type SelectedObject = {
  id: number;
  name: string;
  kind: "object" | "group";
};

/** Complexity-mode segments. Tiers come from libslic3r's per-option mode
 *  metadata (via `passesMode`) — not a curated list. */
const MODE_SEGMENTS: ReadonlyArray<{
  id: "simple" | "advanced" | "expert";
  label: string;
  desc: string;
}> = [
  { id: "simple", label: "Simple", desc: "Everyday essentials — the controls most prints need" },
  { id: "advanced", label: "Advanced", desc: "Common tuning controls for dialing in quality" },
  { id: "expert", label: "Expert", desc: "Every setting exposed — no guardrails" },
];

const RAIL_COLLAPSED_KEY = "n3o.settings.railCollapsed";

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

  // Setting-complexity mode (Simple / Advanced / Expert), persisted.
  const [mode, setMode] = useStoredModeFilter();
  // "Show only modified settings" — the diff toggle. Modified = overridden
  // at the active editing layer (project or object).
  const [showModifiedOnly, setShowModifiedOnly] = useState(false);
  const [search, setSearch] = useState("");
  const [contextLayer, setContextLayer] = useState<ContextLayer>("project");
  const [activeCat, setActiveCat] = useState<string | null>(null);
  // Category rail collapse (icons-only), persisted across reloads like the
  // mode filter — reclaims horizontal space for the setting rows.
  const [railCollapsed, setRailCollapsed] = useState<boolean>(() => {
    try {
      return window.localStorage.getItem(RAIL_COLLAPSED_KEY) === "1";
    } catch {
      return false;
    }
  });
  useEffect(() => {
    try {
      window.localStorage.setItem(RAIL_COLLAPSED_KEY, railCollapsed ? "1" : "0");
    } catch {
      // localStorage may be disabled — ignore.
    }
  }, [railCollapsed]);
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

  // Is this option modified at the active editing layer? Project tab: a
  // project override exists. Object tab: the selected object overrides it.
  // This is the notion the "show modified" filter + tier-tags use.
  const isModified = useCallback(
    (key: string) =>
      contextLayer === "object"
        ? key in objectOverrides
        : key in projectOverrides,
    [contextLayer, objectOverrides, projectOverrides],
  );

  // Apply printer-aware visibility + mode + search to the option list. The
  // core rule: a setting shows if it's in the current mode's tier OR it's
  // been modified — changed settings are never hidden by the mode, so you can
  // always see (and revert) what differs even in Simple, across every
  // category. `showModifiedOnly` then narrows to just the modified ones.
  const visibleOptions = useMemo(() => {
    return options.filter((o) => {
      const modified = isModified(o.key);
      if (showModifiedOnly && !modified) return false;
      return filterRow(o, mode, search, modified);
    });
  }, [options, mode, search, showModifiedOnly, isModified]);

  // Per-mode cumulative counts on the segmented control (printer-applicable
  // options at or below each tier).
  const modeCounts = useMemo(() => {
    const out: Record<"simple" | "advanced" | "expert", number> = {
      simple: 0,
      advanced: 0,
      expert: 0,
    };
    for (const o of options) {
      if (o.hidden) continue;
      for (const m of ["simple", "advanced", "expert"] as const) {
        if (passesMode(o, m)) out[m] += 1;
      }
    }
    return out;
  }, [options]);

  // Total modified settings at the active layer — drives the toggle's count
  // and disabled state.
  const modifiedTotal = useMemo(
    () => options.reduce((n, o) => (isModified(o.key) ? n + 1 : n), 0),
    [options, isModified],
  );
  // If the user filters to modified-only and then clears every override, drop
  // the filter so the list isn't stuck empty.
  useEffect(() => {
    if (showModifiedOnly && modifiedTotal === 0) setShowModifiedOnly(false);
  }, [showModifiedOnly, modifiedTotal]);

  const groups = useMemo(() => categorize(visibleOptions), [visibleOptions]);

  const overriddenKeys = useMemo(() => {
    // Per-category override-count badge follows the active editing tab.
    // Object tab: just the selected object's overrides. Project tab: the
    // union of project overrides + every object's overrides on the plate —
    // from the project vantage the object tier is "below" you, so any
    // object's changes count toward the total (not just the selected one).
    if (contextLayer === "object") return new Set(Object.keys(objectOverrides));
    const out = new Set<string>();
    for (const k of Object.keys(projectOverrides)) out.add(k);
    for (const o of allObjects) for (const k of Object.keys(o.overrides)) out.add(k);
    return out;
  }, [contextLayer, projectOverrides, objectOverrides, allObjects]);

  const counts = useMemo(
    () => categoryCounts(groups, overriddenKeys),
    [groups, overriddenKeys],
  );

  // Cascade ladder hover state. One ladder portal per
  // panel; tracks which row triggered it so the per-row data the
  // ladder reads stays addressable on hover.
  const ladder = useLadderHover();
  const [hoveredKey, setHoveredKey] = useState<string | null>(null);
  const hoveredSchema = useMemo(
    () => visibleOptions.find((o) => o.key === hoveredKey) ?? null,
    [visibleOptions, hoveredKey],
  );

  // The separate `SettingTooltip` popover is folded into the
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
      `[data-cat-id="${CSS.escape(id)}"]`,
    );
    if (el) el.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  const objectTabAvailable = selectedObject != null;

  return (
    <section
      className={`settings-panel${railCollapsed ? " rail-collapsed" : ""}`}
      data-context-layer={contextLayer}
    >
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
                ? selectedObject!.kind === "group"
                  ? `Overrides for every object in ${selectedObject!.name}`
                  : `Per-object overrides for ${selectedObject!.name}`
                : "Select an object on the plate to edit per-object overrides"
            }
          >
            {objectTabAvailable
              ? `${selectedObject!.kind === "group" ? "Group" : "Object"}: ${selectedObject!.name}`
              : "Object"}
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
        <div
          className="search-wrap"
          style={onPluginTab ? { display: "none" } : undefined}
        >
          <div className="search-row">
            <div className="search-input">
              <input
                type="search"
                placeholder="Search settings…"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            {/* Show-only-modified toggle (the diff filter). */}
            <button
              type="button"
              className={`filter-toggle${showModifiedOnly ? " active" : ""}`}
              onClick={() => setShowModifiedOnly((v) => !v)}
              disabled={modifiedTotal === 0 && !showModifiedOnly}
              aria-pressed={showModifiedOnly}
              title={
                modifiedTotal === 0
                  ? `No modified settings on the ${
                      contextLayer === "object" ? "Object" : "Project"
                    } layer yet`
                  : showModifiedOnly
                    ? "Showing only modified settings — click to show all"
                    : `Show only modified settings (${modifiedTotal})`
              }
            >
              <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
                <path
                  d="M1.5 2.5h11l-4.2 5v3.6l-2.6 1.2V7.5L1.5 2.5z"
                  stroke="currentColor"
                  strokeWidth="1.3"
                  strokeLinejoin="round"
                />
              </svg>
              {modifiedTotal > 0 && (
                <span className="filter-toggle-count">{modifiedTotal}</span>
              )}
            </button>
          </div>
          {/* Setting-complexity mode (Simple / Advanced / Expert). A changed
              setting still shows regardless of the mode (see visibleOptions). */}
          <div className="mode-seg" role="tablist" aria-label="Setting complexity">
            {MODE_SEGMENTS.map((m) => (
              <button
                key={m.id}
                type="button"
                role="tab"
                aria-selected={mode === m.id}
                className={`mode-seg-btn${mode === m.id ? " active" : ""}`}
                onClick={() => setMode(m.id)}
                title={m.desc}
              >
                <span className="mode-seg-label">{m.label}</span>
                <span className="mode-seg-count">{modeCounts[m.id]}</span>
              </button>
            ))}
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
          {showModifiedOnly
            ? "No modified settings on this layer — edit a setting to see it here."
            : "No matching settings — try broadening the search."}
          {mode !== "expert" && !showModifiedOnly && (
            <div style={{ marginTop: 6 }}>
              Some settings are hidden by <b>{mode}</b> mode —{" "}
              <button
                type="button"
                className="empty-link"
                onClick={() => setMode("expert")}
              >
                switch to Expert
              </button>
              .
            </div>
          )}
        </div>
      ) : (
        <div className="settings-body">
          <CategorySidebar
            groups={groups}
            counts={counts}
            activeId={activeCat}
            onActivate={jumpToCategory}
            collapsed={railCollapsed}
            onToggleCollapsed={() => setRailCollapsed((v) => !v)}
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
                    outOfMode={!passesMode(opt, mode)}
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
          objectTier={
            contextLayer === "object" && selectedObject != null
              ? {
                  label: `${selectedObject.kind === "group" ? "Group" : "Object"}: ${selectedObject.name}`,
                }
              : null
          }
        />
      )}
        </>
      )}
    </section>
  );
}
