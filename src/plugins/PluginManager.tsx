// PluginManager — the level-scoped plugin list.
//
// Ported from `docs/design/PluginManager.jsx`, adapted to the real,
// leaner `PluginSummary` (no category / glyph / author / summary;
// settings are string/number/bool/enum only — no secret / events
// kinds). Visual structure (rows, tri-state level control, config
// expander, availability pips, provenance + downstream labels) is
// kept faithful; data is sourced entirely from `PluginSummary` plus
// the three override maps in `CascadeSources`.
//
// Presentational + parameterized: the component never calls `invoke`.
// It reads through `pluginCascade` and writes through the `writers`
// the mounting surface (Global modal / Project modal / Plate panel)
// supplies, each pre-bound to its level + (for plate) its plateId.

import { useState } from "react";
import type { PluginSummary, SettingSummary } from "./pluginCommands";
import {
  PLUGIN_LEVEL_META,
  PLUGIN_LEVEL_ORDER,
  type CascadeSources,
  type PluginLevel,
  type RawActivation,
  type TypedSettingValue,
  configOwnerLevel,
  downstreamOverride,
  isRootLevel,
  readActivation,
  readSettingRaw,
  resolveActivation,
  resolveSettings,
} from "./pluginCascade";

/** Writer callbacks, pre-bound by the mounting surface to its level
 *  (and plateId for the plate surface). Keeps `PluginManager` free of
 *  `invoke` and level-routing. */
export interface PluginWriters {
  /** Set this level's activation for a plugin. `"inherit"` clears the
   *  override (no-op clear at the global root, which is binary). */
  setActivation: (plugin: PluginSummary, value: RawActivation) => void;
  /** Set one of the plugin's settings at this level. */
  setSetting: (
    plugin: PluginSummary,
    setting: SettingSummary,
    value: TypedSettingValue,
  ) => void;
  /** Clear all of the plugin's setting overrides at this level
   *  (→ inherit). The global root has nothing to clear; surfaces may
   *  no-op it. */
  clearSettings: (plugin: PluginSummary) => void;
  /** Reload the plugin from disk. */
  reload?: (plugin: PluginSummary) => void;
}

export interface PluginManagerProps {
  level: PluginLevel;
  plugins: PluginSummary[];
  sources: CascadeSources;
  writers: PluginWriters;
  /** Render-only (no controls fire). */
  readOnly?: boolean;
  /** Plate identity, for the intro label on the plate surface. */
  plateName?: string | null;
}

// ── Field control ─────────────────────────────────────────────────

interface PluginFieldProps {
  setting: SettingSummary;
  /** Current effective value (string form). */
  value: string;
  onChange: (value: TypedSettingValue) => void;
  disabled: boolean;
  /** True when this level explicitly set this field. */
  overridden: boolean;
}

function PluginField({
  setting,
  value,
  onChange,
  disabled,
  overridden,
}: PluginFieldProps): React.JSX.Element {
  const label = (
    <label className={overridden ? "is-override" : ""}>
      {setting.label ?? setting.key}
      {overridden && <span className="plg-field-tag">set here</span>}
    </label>
  );

  if (setting.kind === "bool") {
    const on = value === "true" || value === "1";
    return (
      <div className="plg-field plg-toggle-row">
        {label}
        <button
          type="button"
          role="switch"
          aria-checked={on}
          aria-label={setting.label ?? setting.key}
          disabled={disabled}
          className={`val-toggle${on ? " on" : ""}${disabled ? " is-disabled" : ""}`}
          onClick={() => onChange(!on)}
        />
      </div>
    );
  }

  if (setting.kind === "enum") {
    return (
      <div className="plg-field">
        {label}
        <select
          className="val-select"
          value={value}
          disabled={disabled}
          aria-label={setting.label ?? setting.key}
          onChange={(e) => onChange(e.target.value)}
        >
          {setting.values.map((opt) => (
            <option key={opt} value={opt}>
              {opt}
            </option>
          ))}
        </select>
      </div>
    );
  }

  if (setting.kind === "number") {
    return (
      <div className="plg-field">
        {label}
        <div className="val-wrap">
          <input
            className="val-input"
            type="number"
            value={value}
            step="any"
            disabled={disabled}
            onChange={(e) =>
              onChange(e.target.value === "" ? "" : Number(e.target.value))
            }
          />
        </div>
      </div>
    );
  }

  // string
  return (
    <div className="plg-field">
      {label}
      <div className="apm-name-input">
        <input
          type="text"
          value={value}
          autoComplete="off"
          spellCheck={false}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
        />
      </div>
    </div>
  );
}

// ── Tri-state level control ───────────────────────────────────────

interface LevelControlProps {
  isRoot: boolean;
  value: RawActivation;
  onChange: (value: RawActivation) => void;
  readOnly: boolean;
}

function LevelControl({
  isRoot,
  value,
  onChange,
  readOnly,
}: LevelControlProps): React.JSX.Element {
  const segs: ReadonlyArray<{ v: RawActivation; key: string; label: string }> =
    isRoot
      ? [
          { v: "on", key: "on", label: "On" },
          { v: "off", key: "off", label: "Off" },
        ]
      : [
          { v: undefined, key: "inherit", label: "Inherit" },
          { v: "on", key: "on", label: "On" },
          { v: "off", key: "off", label: "Off" },
        ];
  const cur = value === "on" ? "on" : value === "off" ? "off" : "inherit";
  return (
    <div className={`plev-seg${readOnly ? " readonly" : ""}`} role="group">
      {segs.map((s) => (
        <button
          key={s.key}
          type="button"
          className={`plev-seg-btn ${cur === s.key ? "active" : ""} ${s.key}`}
          onClick={() => !readOnly && onChange(s.v)}
        >
          {s.label}
        </button>
      ))}
    </div>
  );
}

// ── Availability pips ─────────────────────────────────────────────

function LevelBadges({
  plugin,
  activeLevel,
}: {
  plugin: PluginSummary;
  activeLevel: PluginLevel;
}): React.JSX.Element {
  return (
    <span
      className="plg-avail"
      title={`Available at: ${plugin.scopes
        .map((l) => PLUGIN_LEVEL_META[l as PluginLevel]?.label ?? l)
        .join(", ")}`}
    >
      {PLUGIN_LEVEL_ORDER.map((l) => {
        const on = plugin.scopes.includes(l);
        return (
          <span
            key={l}
            className={`plg-avail-pip ${on ? "on" : "off"} ${l === activeLevel ? "here" : ""}`}
          >
            {PLUGIN_LEVEL_META[l].short}
          </span>
        );
      })}
    </span>
  );
}

// ── Secondary text (no `summary` in the backend) ──────────────────

/** Best-effort one-liner from the lean PluginSummary: hooks first,
 *  then printer scoping. */
function pluginSubtitle(plugin: PluginSummary): string {
  const parts: string[] = [];
  if (plugin.hooks.length > 0) {
    parts.push(`hooks: ${plugin.hooks.join(", ")}`);
  }
  if (plugin.printers && plugin.printers.length > 0) {
    parts.push(`printers: ${plugin.printers.join(", ")}`);
  }
  return parts.join(" · ");
}

// ── One plugin row ────────────────────────────────────────────────

interface PluginRowProps {
  plugin: PluginSummary;
  level: PluginLevel;
  sources: CascadeSources;
  writers: PluginWriters;
  readOnly: boolean;
}

function PluginRow({
  plugin,
  level,
  sources,
  writers,
  readOnly,
}: PluginRowProps): React.JSX.Element {
  const [open, setOpen] = useState(false);

  const root = isRootLevel(plugin, level);
  const here = readActivation(plugin, level, sources);
  const resolved = resolveActivation(plugin, level, sources);
  const downstream = downstreamOverride(plugin, level, sources);

  // Config is overridable only where the plugin is explicitly On here.
  const editableHere = here === "on" && !readOnly;
  const effective = resolveSettings(plugin, level, sources);
  const owner = configOwnerLevel(plugin, level, sources);
  const subtitle = pluginSubtitle(plugin);
  const hasSettings = plugin.settings.length > 0;
  // Does this level carry any explicit setting overrides?
  const hasLocalSettings = plugin.settings.some(
    (s) => readSettingRaw(plugin, level, s, sources) !== undefined,
  );

  // Provenance label.
  let prov: string;
  if (resolved.source === "default") {
    prov = "Default · off";
  } else if (resolved.source === level) {
    prov = `${resolved.enabled ? "On" : "Off"} · set here`;
  } else {
    prov = `${resolved.enabled ? "On" : "Off"} · from ${PLUGIN_LEVEL_META[resolved.source].label}`;
  }

  return (
    <div className={`plg-row2 ${resolved.enabled ? "is-on" : "is-off"}`}>
      <div className="plg-row2-main">
        <div className="plg-row2-text">
          <div className="plg-row2-head">
            <span className="plg-row2-name">{plugin.name}</span>
            <span className="plg-avail-pip on" title="Version">
              v{plugin.version}
            </span>
            <LevelBadges plugin={plugin} activeLevel={level} />
          </div>
          {subtitle && <div className="plg-row2-sum">{subtitle}</div>}
          {plugin.last_error && (
            <div className="plg-config-note" role="alert">
              {plugin.last_error}
            </div>
          )}
          <div className="plg-row2-meta">
            <span
              className={`plg-prov ${resolved.enabled ? "on" : "off"} ${resolved.source === level ? "set" : ""}`}
            >
              <span className="plg-prov-dot" />
              {prov}
            </span>
            {downstream && (
              <span
                className="plg-prov-down"
                title={`Overridden at ${PLUGIN_LEVEL_META[downstream].label} level`}
              >
                overridden at {PLUGIN_LEVEL_META[downstream].label.toLowerCase()}
              </span>
            )}
            {hasSettings && (
              <button
                type="button"
                className={`plg-config-toggle ${open ? "open" : ""}`}
                onClick={() => setOpen((o) => !o)}
              >
                <svg width="9" height="9" viewBox="0 0 10 10" fill="none">
                  <path
                    d="M2 3.5l3 3 3-3"
                    stroke="currentColor"
                    strokeWidth="1.4"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
                Configure
              </button>
            )}
            {writers.reload && !readOnly && (
              <button
                type="button"
                className="plg-config-toggle"
                onClick={() => writers.reload?.(plugin)}
                title="Reload plugin from disk"
              >
                Reload
              </button>
            )}
          </div>
        </div>
        <LevelControl
          isRoot={root}
          value={here}
          onChange={(v) => !readOnly && writers.setActivation(plugin, v)}
          readOnly={readOnly}
        />
      </div>

      {open && hasSettings && (
        <div className={`plg-row2-config ${editableHere ? "" : "locked"}`}>
          <div className="plg-config-bar">
            <span className="plg-config-status">
              {editableHere
                ? "Editable here"
                : owner
                  ? `Inherited · configured at ${PLUGIN_LEVEL_META[owner].label}`
                  : "Defaults"}
            </span>
            {editableHere && hasLocalSettings && (
              <button
                type="button"
                className="plg-config-reset"
                onClick={() => writers.clearSettings(plugin)}
              >
                Reset to inherited
              </button>
            )}
          </div>
          {!editableHere && (
            <div className="plg-config-note">
              {readOnly
                ? "Read-only preview."
                : here === "off"
                  ? `Off at ${PLUGIN_LEVEL_META[level].label} — settings don't apply here.`
                  : `Turn this plugin On at ${PLUGIN_LEVEL_META[level].label} to override its settings here.`}
            </div>
          )}
          {plugin.settings.map((s) => (
            <PluginField
              key={s.key}
              setting={s}
              value={effective[s.key] ?? s.default}
              overridden={
                editableHere &&
                readSettingRaw(plugin, level, s, sources) !== undefined
              }
              disabled={!editableHere}
              onChange={(v) => writers.setSetting(plugin, s, v)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// ── Manager ───────────────────────────────────────────────────────

export function PluginManager({
  level,
  plugins,
  sources,
  writers,
  readOnly = false,
  plateName,
}: PluginManagerProps): React.JSX.Element {
  const meta = PLUGIN_LEVEL_META[level];
  const available = plugins.filter((p) => p.scopes.includes(level));
  const unavailable = plugins.filter((p) => !p.scopes.includes(level));

  const introTail =
    level === "global"
      ? "the baseline for every project. Projects and plates inherit these unless they override."
      : level === "project"
        ? "inherits from Global. Anything set here overrides Global for this project and cascades to its plates."
        : "inherits from Project. Anything set here applies to just this plate and overrides everything above.";

  return (
    <div className="plg-manager">
      <div
        className="plg-intro"
        style={{ "--lvl-hue": meta.hue } as React.CSSProperties}
      >
        <span className="plg-intro-dot" />
        <div className="plg-intro-text">
          <b>
            {meta.label} plugins
            {level === "plate" && plateName ? ` · ${plateName}` : ""}
          </b>
          <span> — {introTail}</span>
        </div>
      </div>

      <div className="plg-group">
        {available.length === 0 ? (
          <div className="plg-group-label">No plugins available here</div>
        ) : (
          available.map((p) => (
            <PluginRow
              key={p.name}
              plugin={p}
              level={level}
              sources={sources}
              writers={writers}
              readOnly={readOnly}
            />
          ))
        )}
      </div>

      {unavailable.length > 0 && (
        <div className="plg-unavailable">
          <div className="plg-group-label">Not available at this level</div>
          <div className="plg-unavailable-list">
            {unavailable.map((p) => (
              <span
                className="plg-unavailable-item"
                key={p.name}
                title={`${p.name} can only be enabled at: ${p.scopes
                  .map((l) => PLUGIN_LEVEL_META[l as PluginLevel]?.label ?? l)
                  .join(", ")}`}
              >
                {p.name}
              </span>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
