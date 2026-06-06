// PluginManager.jsx — multi-level, cascading plugin enablement.
//
// THE MODEL
//   Plugins can be enabled at three levels that cascade:
//       Global  →  Project  →  Plate
//   Each plugin self-declares which levels it's *available* at (`levels`).
//   At each available level enablement is tri-state:
//       "on" | "off" | <unset = inherit from the level above>
//   The first available level for a plugin is its ROOT (no inherit — unset
//   there means the built-in default, which is off unless `defaultGlobal`).
//   A lower level always overrides a higher one, so a plugin enabled globally
//   can be explicitly switched off on a single plate.
//
// One <PluginManager> renders the list for a given level; <PluginsModal> wraps
// it for the menu-launched Global/Project surfaces, while the Plate surface
// embeds <PluginManager> directly inside the settings panel.

const PLUGIN_LEVEL_ORDER = ["global", "project", "plate"];

const PLUGIN_LEVEL_META = {
  global:  { label: "Global",  short: "G", blurb: "Every project on this machine", hue: 215 },
  project: { label: "Project", short: "P", blurb: "This .3mf file",                hue: 285 },
  plate:   { label: "Plate",   short: "PL", blurb: "Just the active plate",        hue: 340 },
};

const PLUGIN_CATALOG = [
  {
    id: "mesh_doctor",
    name: "Mesh Doctor",
    category: "Geometry",
    glyph: "△",
    author: "n3o labs",
    version: "3.0.2",
    levels: ["global", "project", "plate"],
    defaultGlobal: true,
    summary: "Repairs non-manifold edges and flipped normals before slicing.",
    fields: [
      { key: "strength", kind: "select", label: "Repair strength", options: ["Conservative", "Balanced", "Aggressive"], default: "Balanced" },
      { key: "fillHoles", kind: "toggle", label: "Fill holes", default: true },
    ],
  },
  {
    id: "arc_welder",
    name: "Arc Welder",
    category: "Post-processing",
    glyph: "◜",
    author: "n3o labs",
    version: "2.1.0",
    levels: ["global", "project", "plate"],
    summary: "Replaces short G1 runs with smooth G2/G3 arcs — smaller files, cleaner curves.",
    fields: [
      { key: "tolerance", kind: "number", label: "Path tolerance", unit: "mm", default: 0.05, min: 0.01, max: 0.5, step: 0.01 },
      { key: "firmware", kind: "select", label: "Target firmware", options: ["Marlin 2.x", "Klipper", "RepRapFirmware"], default: "Marlin 2.x" },
    ],
  },
  {
    id: "octoprint",
    name: "OctoPrint Upload",
    category: "Output",
    glyph: "☁",
    author: "OctoPrint",
    version: "0.9.5",
    levels: ["global", "project", "plate"],
    summary: "Sends sliced G-code to an OctoPrint server, optionally starting the print.",
    fields: [
      { key: "url", kind: "text", label: "Server URL", placeholder: "http://octopi.local" },
      { key: "apiKey", kind: "secret", label: "API key" },
      { key: "autostart", kind: "toggle", label: "Start print after upload", default: false },
    ],
  },
  {
    id: "pushover",
    name: "Pushover Notify",
    category: "Notifications",
    glyph: "✦",
    author: "community",
    version: "1.0.1",
    levels: ["global", "project"],
    summary: "Push a phone notification on slice and print milestones. Account-wide — no per-plate setup.",
    fields: [
      { key: "userKey", kind: "secret", label: "User key" },
      { key: "events", kind: "events", label: "Notify me when…", options: ["Slice complete", "Print finished", "Error / fault"], default: ["Slice complete", "Print finished"] },
    ],
  },
  {
    id: "timelapse",
    name: "Time-lapse Hook",
    category: "Output",
    glyph: "▦",
    author: "community",
    version: "1.1.0",
    levels: ["project", "plate"],
    summary: "Adds a per-layer snapshot trigger so OctoPrint/Moonraker can build a time-lapse.",
    fields: [
      { key: "park", kind: "toggle", label: "Park toolhead for each frame", default: true },
    ],
  },
  {
    id: "filament_change",
    name: "Filament Change (M600)",
    category: "Post-processing",
    glyph: "⇅",
    author: "community",
    version: "1.4.0",
    levels: ["plate"],
    summary: "Inserts an M600 color-change pause at chosen layers. Layers are specific to a plate.",
    fields: [
      { key: "layers", kind: "text", label: "Pause at layers", placeholder: "e.g. 24, 60, 142" },
      { key: "beep", kind: "toggle", label: "Beep when paused", default: true },
    ],
  },
  {
    id: "cal_tower",
    name: "Calibration Tower",
    category: "Calibration",
    glyph: "≣",
    author: "n3o labs",
    version: "2.3.0",
    levels: ["plate"],
    summary: "Generates a parametric test tower banded over the active plate.",
    fields: [
      { key: "type", kind: "select", label: "Tower type", options: ["Temperature", "Retraction", "Flow"], default: "Temperature" },
      { key: "start", kind: "number", label: "Start value", default: 220, step: 1 },
      { key: "end", kind: "number", label: "End value", default: 190, step: 1 },
    ],
  },
];

const PLUGIN_CATEGORY_ORDER = ["Geometry", "Post-processing", "Output", "Calibration", "Notifications"];

// ───────── State builders ─────────

function defaultPluginEnablement() {
  const global = {};
  PLUGIN_CATALOG.forEach(p => {
    if (p.levels.includes("global") && p.defaultGlobal) global[p.id] = "on";
  });
  return { global, project: {}, plate: {} };
}

function defaultPluginConfig() {
  // Config overrides are stored per level (mirrors enablement). Field defaults
  // live in the catalog and are applied at resolve time, so these start empty.
  return { global: {}, project: {}, plate: {} };
}

// Catalog field defaults as a plain object for a plugin.
function pluginConfigDefaults(p) {
  const v = {};
  (p.fields || []).forEach(f => {
    v[f.key] = f.default !== undefined
      ? (Array.isArray(f.default) ? [...f.default] : f.default)
      : (f.kind === "toggle" ? false : f.kind === "events" ? [] : "");
  });
  return v;
}

// Levels a plugin participates in, in cascade order.
function pluginLevels(p) {
  return PLUGIN_LEVEL_ORDER.filter(l => p.levels.includes(l));
}

// Read the raw stored override for a plugin at a level ("on"|"off"|undefined).
function readLevel(enablement, level, pluginId, plateId) {
  if (level === "plate") return enablement.plate?.[plateId]?.[pluginId];
  return enablement[level]?.[pluginId];
}

// Read a plugin's stored config overrides at one level (object or undefined).
function readConfigLevel(config, level, pluginId, plateId) {
  if (level === "plate") return config.plate?.[plateId]?.[pluginId];
  return config[level]?.[pluginId];
}

// Resolve a plugin's effective config as seen at `uptoLevel`. Starts from the
// catalog defaults, then layers on overrides from each level — BUT only from
// levels where the plugin is explicitly enabled ("on"). A level set to inherit
// or off contributes nothing to config, because config is only overridable
// where you've deliberately turned the plugin on.
function resolvePluginConfig(enablement, config, p, plateId, uptoLevel) {
  const order = pluginLevels(p);
  const cap = uptoLevel ? PLUGIN_LEVEL_ORDER.indexOf(uptoLevel) : Infinity;
  const result = pluginConfigDefaults(p);
  order.forEach(lvl => {
    if (PLUGIN_LEVEL_ORDER.indexOf(lvl) > cap) return;
    if (readLevel(enablement, lvl, p.id, plateId) !== "on") return;
    const stored = readConfigLevel(config, lvl, p.id, plateId);
    if (stored) Object.keys(stored).forEach(k => { result[k] = stored[k]; });
  });
  return result;
}

// The deepest level at/above `uptoLevel` where the plugin is explicitly on —
// i.e. the level that "owns" the effective config. null if none are on.
function configOwnerLevel(enablement, p, plateId, uptoLevel) {
  const order = pluginLevels(p);
  const cap = PLUGIN_LEVEL_ORDER.indexOf(uptoLevel);
  let owner = null;
  order.forEach(lvl => {
    if (PLUGIN_LEVEL_ORDER.indexOf(lvl) <= cap && readLevel(enablement, lvl, p.id, plateId) === "on") owner = lvl;
  });
  return owner;
}

// Resolve the effective state of a plugin as seen *at* `uptoLevel`, walking
// the cascade. Returns { enabled, source } where source is the level that
// decided it, or "default".
function resolvePlugin(enablement, p, plateId, uptoLevel) {
  const order = pluginLevels(p);
  const cap = uptoLevel ? PLUGIN_LEVEL_ORDER.indexOf(uptoLevel) : Infinity;
  let enabled = false, source = "default";
  order.forEach((lvl, i) => {
    if (PLUGIN_LEVEL_ORDER.indexOf(lvl) > cap) return;
    const raw = readLevel(enablement, lvl, p.id, plateId);
    if (raw === "on") { enabled = true; source = lvl; }
    else if (raw === "off") { enabled = false; source = lvl; }
    else if (i === 0) { enabled = !!p.defaultGlobal && lvl === "global"; source = enabled ? "default" : "default"; }
    // else: inherit — carry forward
  });
  return { enabled, source };
}

// Does any level BELOW `level` carry an explicit override for this plugin?
function hasDownstreamOverride(enablement, p, level) {
  const order = pluginLevels(p);
  const idx = order.indexOf(level);
  for (let i = idx + 1; i < order.length; i++) {
    const lvl = order[i];
    if (lvl === "plate") {
      const plates = enablement.plate || {};
      if (Object.values(plates).some(pp => pp && pp[p.id] !== undefined)) return lvl;
    } else if (enablement[lvl]?.[p.id] !== undefined) {
      return lvl;
    }
  }
  return null;
}

function countPluginsEnabledAtLevel(enablement, level, plateId) {
  return PLUGIN_CATALOG.filter(p => p.levels.includes(level))
    .filter(p => resolvePlugin(enablement, p, plateId, level).enabled).length;
}

// ───────── Config field control ─────────

function PluginField({ field, value, onChange, disabled, overridden }) {
  const lbl = (
    <label className={overridden ? "is-override" : ""}>
      {field.label}
      {overridden && <span className="plg-field-tag">set here</span>}
    </label>
  );
  if (field.kind === "toggle") {
    return (
      <div className="plg-field plg-toggle-row">
        {lbl}
        <div className={`val-toggle ${value ? "on" : ""} ${disabled ? "disabled" : ""}`}
             onClick={() => !disabled && onChange(!value)} role="switch" aria-checked={!!value}/>
      </div>
    );
  }
  if (field.kind === "select") {
    return (
      <div className="plg-field">
        {lbl}
        <select className="val-select" value={value} disabled={disabled} onChange={(e) => onChange(e.target.value)}>
          {field.options.map(o => <option key={o} value={o}>{o}</option>)}
        </select>
      </div>
    );
  }
  if (field.kind === "number") {
    return (
      <div className="plg-field">
        {lbl}
        <div className="psm-limit-input">
          <input type="number" value={value ?? ""} step={field.step ?? 1} min={field.min} max={field.max}
                 disabled={disabled}
                 onChange={(e) => onChange(e.target.value === "" ? "" : Number(e.target.value))}/>
          {field.unit && <span className="psm-limit-unit">{field.unit}</span>}
        </div>
      </div>
    );
  }
  if (field.kind === "events") {
    const sel = Array.isArray(value) ? value : [];
    return (
      <div className="plg-field">
        {lbl}
        <div className="plg-event-chips">
          {field.options.map(opt => (
            <button key={opt} type="button" disabled={disabled}
                    className={`plg-event-chip ${sel.includes(opt) ? "on" : ""}`}
                    onClick={() => onChange(sel.includes(opt) ? sel.filter(o => o !== opt) : [...sel, opt])}>
              {opt}
            </button>
          ))}
        </div>
      </div>
    );
  }
  return (
    <div className="plg-field">
      {lbl}
      <div className="apm-name-input">
        <input type={field.kind === "secret" ? "password" : "text"} value={value ?? ""}
               placeholder={field.placeholder || ""} autoComplete="off" spellCheck={false} disabled={disabled}
               onChange={(e) => onChange(e.target.value)}/>
      </div>
    </div>
  );
}

// ───────── Level enablement control (segmented tri-state) ─────────

function LevelControl({ isRoot, value, onChange, readOnly }) {
  // value: "on" | "off" | "inherit"
  const segs = isRoot
    ? [{ v: "on", label: "On" }, { v: "off", label: "Off" }]
    : [{ v: "inherit", label: "Inherit" }, { v: "on", label: "On" }, { v: "off", label: "Off" }];
  const cur = value === "on" ? "on" : value === "off" ? "off" : (isRoot ? "off" : "inherit");
  return (
    <div className={`plev-seg ${readOnly ? "readonly" : ""}`} role="group">
      {segs.map(s => (
        <button key={s.v} type="button"
                className={`plev-seg-btn ${cur === s.v ? "active" : ""} ${s.v}`}
                onClick={() => !readOnly && onChange(s.v)}>
          {s.label}
        </button>
      ))}
    </div>
  );
}

// ───────── Availability badges ─────────

function LevelBadges({ plugin, activeLevel }) {
  return (
    <span className="plg-avail" title={`Available at: ${plugin.levels.map(l => PLUGIN_LEVEL_META[l].label).join(", ")}`}>
      {PLUGIN_LEVEL_ORDER.map(l => {
        const on = plugin.levels.includes(l);
        return (
          <span key={l}
                className={`plg-avail-pip ${on ? "on" : "off"} ${l === activeLevel ? "here" : ""}`}>
            {PLUGIN_LEVEL_META[l].short}
          </span>
        );
      })}
    </span>
  );
}

// ───────── One plugin row ─────────

function PluginRow({ plugin, level, plateId, enablement, setEnablement, config, setConfig, readOnly }) {
  const { useState } = React;
  const [open, setOpen] = useState(false);

  const order = pluginLevels(plugin);
  const isRoot = order[0] === level;
  const raw = readLevel(enablement, level, plugin.id, plateId);
  const here = raw === "on" ? "on" : raw === "off" ? "off" : "inherit";

  const resolved = resolvePlugin(enablement, plugin, plateId, level);
  const downstream = hasDownstreamOverride(enablement, plugin, level);

  const setLevelValue = (v) => {
    setEnablement(prev => {
      const next = { ...prev };
      if (level === "plate") {
        const plates = { ...(next.plate || {}) };
        const cur = { ...(plates[plateId] || {}) };
        if (v === "inherit") delete cur[plugin.id]; else cur[plugin.id] = v;
        plates[plateId] = cur;
        next.plate = plates;
      } else {
        const lvl = { ...(next[level] || {}) };
        if (v === "inherit") delete lvl[plugin.id]; else lvl[plugin.id] = v;
        next[level] = lvl;
      }
      return next;
    });
  };

  const setFieldValue = (key, val) => {
    setConfig(prev => {
      const next = { ...prev };
      if (level === "plate") {
        const plates = { ...(next.plate || {}) };
        const cur = { ...(plates[plateId] || {}) };
        cur[plugin.id] = { ...(cur[plugin.id] || {}), [key]: val };
        plates[plateId] = cur;
        next.plate = plates;
      } else {
        const lvl = { ...(next[level] || {}) };
        lvl[plugin.id] = { ...(lvl[plugin.id] || {}), [key]: val };
        next[level] = lvl;
      }
      return next;
    });
  };

  const resetFieldOverrides = () => {
    setConfig(prev => {
      const next = { ...prev };
      if (level === "plate") {
        const plates = { ...(next.plate || {}) };
        const cur = { ...(plates[plateId] || {}) };
        delete cur[plugin.id];
        plates[plateId] = cur;
        next.plate = plates;
      } else {
        const lvl = { ...(next[level] || {}) };
        delete lvl[plugin.id];
        next[level] = lvl;
      }
      return next;
    });
  };

  // Config is only OVERRIDABLE where the plugin is explicitly On at this level.
  const editableHere = here === "on" && !readOnly;
  const effectiveConfig = resolvePluginConfig(enablement, config, plugin, plateId, level);
  const localCfg = readConfigLevel(config, level, plugin.id, plateId) || {};
  const owner = configOwnerLevel(enablement, plugin, plateId, level);

  // Provenance label.
  let prov;
  if (resolved.source === "default") prov = "Default · off";
  else if (resolved.source === level) prov = `${resolved.enabled ? "On" : "Off"} · set here`;
  else prov = `${resolved.enabled ? "On" : "Off"} · from ${PLUGIN_LEVEL_META[resolved.source].label}`;

  return (
    <div className={`plg-row2 ${resolved.enabled ? "is-on" : "is-off"}`}>
      <div className="plg-row2-main">
        <div className="plg-row2-text">
          <div className="plg-row2-head">
            <span className="plg-row2-name">{plugin.name}</span>
            <LevelBadges plugin={plugin} activeLevel={level}/>
          </div>
          <div className="plg-row2-sum">{plugin.summary}</div>
          <div className="plg-row2-meta">
            <span className={`plg-prov ${resolved.enabled ? "on" : "off"} ${resolved.source === level ? "set" : ""}`}>
              <span className="plg-prov-dot"/>{prov}
            </span>
            {downstream && (
              <span className="plg-prov-down" title={`Overridden at ${PLUGIN_LEVEL_META[downstream].label} level`}>
                overridden at {PLUGIN_LEVEL_META[downstream].label.toLowerCase()}
              </span>
            )}
            {plugin.fields && plugin.fields.length > 0 && (
              <button type="button" className={`plg-config-toggle ${open ? "open" : ""}`} onClick={() => setOpen(o => !o)}>
                <svg width="9" height="9" viewBox="0 0 10 10" fill="none"><path d="M2 3.5l3 3 3-3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/></svg>
                Configure
              </button>
            )}
          </div>
        </div>
        <LevelControl isRoot={isRoot} value={here} onChange={setLevelValue} readOnly={readOnly}/>
      </div>

      {open && plugin.fields && plugin.fields.length > 0 && (
        <div className={`plg-row2-config ${editableHere ? "" : "locked"}`}>
          <div className="plg-config-bar">
            <span className="plg-config-status">
              {editableHere
                ? "Editable here"
                : owner
                  ? `Inherited · configured at ${PLUGIN_LEVEL_META[owner].label}`
                  : "Defaults"}
            </span>
            {editableHere && Object.keys(localCfg).length > 0 && (
              <button type="button" className="plg-config-reset" onClick={resetFieldOverrides}>
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
          {plugin.fields.map(f => (
            <PluginField key={f.key} field={f} value={effectiveConfig[f.key]}
                         overridden={editableHere && localCfg[f.key] !== undefined}
                         onChange={(v) => setFieldValue(f.key, v)} disabled={!editableHere}/>
          ))}
        </div>
      )}
    </div>
  );
}

// ───────── Manager (level-scoped list) ─────────

function PluginManager({ level, plateId, plateName, enablement, setEnablement, config, setConfig, readOnly }) {
  const meta = PLUGIN_LEVEL_META[level];
  const available = PLUGIN_CATALOG.filter(p => p.levels.includes(level));
  const groups = PLUGIN_CATEGORY_ORDER
    .map(cat => ({ cat, items: available.filter(p => p.category === cat) }))
    .filter(g => g.items.length);
  const unavailable = PLUGIN_CATALOG.filter(p => !p.levels.includes(level));

  return (
    <div className="plg-manager">
      <div className="plg-intro" style={{ "--lvl-hue": meta.hue }}>
        <span className="plg-intro-dot"/>
        <div className="plg-intro-text">
          <b>{meta.label} plugins{level === "plate" && plateName ? ` · ${plateName}` : ""}</b>
          <span> — {level === "global"
            ? "the baseline for every project. Projects and plates inherit these unless they override."
            : level === "project"
              ? "inherits from Global. Anything set here overrides Global for this project and cascades to its plates."
              : "inherits from Project. Anything set here applies to just this plate and overrides everything above."}</span>
        </div>
      </div>

      {groups.map(g => (
        <div className="plg-group" key={g.cat}>
          <div className="plg-group-label">{g.cat}</div>
          {g.items.map(p => (
            <PluginRow key={p.id} plugin={p} level={level} plateId={plateId}
                       enablement={enablement} setEnablement={setEnablement}
                       config={config} setConfig={setConfig} readOnly={readOnly}/>
          ))}
        </div>
      ))}

      {unavailable.length > 0 && (
        <div className="plg-unavailable">
          <div className="plg-group-label">Not available at this level</div>
          <div className="plg-unavailable-list">
            {unavailable.map(p => (
              <span className="plg-unavailable-item" key={p.id}
                    title={`${p.name} can only be enabled at: ${p.levels.map(l => PLUGIN_LEVEL_META[l].label).join(", ")}`}>
                {p.name}
              </span>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ───────── Modal wrapper (Global / Project surfaces) ─────────

function PluginsModal({ level, projectName, plateName, plateId, enablement, setEnablement, config, setConfig, onClose }) {
  const { useEffect } = React;
  useEffect(() => {
    const onKey = (e) => { if (e.key === "Escape") { e.stopPropagation(); onClose(); } };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const meta = PLUGIN_LEVEL_META[level];
  const count = countPluginsEnabledAtLevel(enablement, level, plateId);
  const total = PLUGIN_CATALOG.filter(p => p.levels.includes(level)).length;

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="plugins-modal" onClick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-labelledby="plg-title">
        <header className="plg-header">
          <div className="plg-header-mark" style={{ "--lvl-hue": meta.hue }} aria-hidden="true">
            <svg width="19" height="19" viewBox="0 0 16 16" fill="none">
              <path d="M6 2v2M10 2v2M4 4h8v3a4 4 0 0 1-4 4 4 4 0 0 1-4-4V4zM8 11v3" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round"/>
            </svg>
          </div>
          <div className="plg-header-text">
            <h2 id="plg-title">{meta.label} plugins</h2>
            <p>{level === "global" ? "Applies to every project on this machine." : `Scoped to ${projectName}. Inherits from Global.`}</p>
          </div>
          <button className="apm-close" onClick={onClose} aria-label="Close" title="Close (Esc)">
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none"><path d="M3 3l8 8M11 3l-8 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/></svg>
          </button>
        </header>

        <div className="plg-modal-body">
          <PluginManager level={level} plateId={plateId} plateName={plateName}
                         enablement={enablement} setEnablement={setEnablement}
                         config={config} setConfig={setConfig}/>
        </div>

        <footer className="plg-footer">
          <span className="plg-foot-hint">Lower levels override higher ones. Unset = inherit.</span>
          <div className="plg-footer-right">
            <span className="plg-enabled-count">{count} of {total} active here</span>
            <button className="apm-btn primary" onClick={onClose} type="button">Done</button>
          </div>
        </footer>
      </div>
    </div>
  );
}

Object.assign(window, {
  PLUGIN_CATALOG,
  PLUGIN_LEVEL_META,
  defaultPluginEnablement,
  defaultPluginConfig,
  resolvePlugin,
  resolvePluginConfig,
  countPluginsEnabledAtLevel,
  PluginManager,
  PluginsModal,
});
