// SettingsPanel.jsx — categorized scrolling settings list with
// instant search, category jump-rail, and value-cascade accountability.

const { useState: useSPS, useMemo: useSPM, useRef: useSPR, useEffect: useSPE, useCallback: useSPC } = React;

const {
  CASCADE_LAYERS, LAYER_BY_ID,
  CATEGORIES, ALL_SETTINGS,
  resolveValue, getOverriddenLayers,
  computeSlotIds, slotShortLabel, slotLongLabel,
} = window.SLICER_DATA;

// Connection-indicator tooltip copy, keyed by status.
const CONN_LABELS = {
  none:       "No connection configured",
  connecting: "Connecting…",
  connected:  "Connected",
  failed:     "Connection failed",
};

// tiny chevron used on dropdown chips
const ChevronChip = () => (
  <svg width="9" height="9" viewBox="0 0 10 10" fill="none" style={{ opacity: 0.55, flexShrink: 0 }}>
    <path d="M2 4l3 3 3-3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
  </svg>
);

// Print-quality presets. Each maps to a base layer height; selecting one is
// the coarse "how fine do you want this print" control that sits above the
// 800+ granular settings below. Ordered fastest → finest.
const QUALITY_PROFILES = [
  { id: "draft",     name: "Draft",      layer: "0.28", note: "Fast" },
  { id: "standard",  name: "Standard",   layer: "0.20", note: "Balanced" },
  { id: "fine",      name: "Fine",       layer: "0.12", note: "Detailed" },
  { id: "superfine", name: "Extra Fine", layer: "0.08", note: "Slowest" },
];

// ───────── fuzzy match (very small implementation) ─────────
function fuzzyScore(needle, hay) {
  if (!needle) return 1;
  needle = needle.toLowerCase();
  hay = hay.toLowerCase();
  if (hay.includes(needle)) return 1 - (hay.indexOf(needle) / hay.length) * 0.3;
  let ni = 0; let score = 0; let lastIdx = -1;
  for (let i = 0; i < hay.length && ni < needle.length; i++) {
    if (hay[i] === needle[ni]) {
      score += 1 - (lastIdx === -1 ? 0 : (i - lastIdx - 1) * 0.05);
      lastIdx = i;
      ni++;
    }
  }
  return ni === needle.length ? Math.max(0, score / needle.length * 0.6) : 0;
}

// ───────── Cascade Ladder (hover) ─────────
// Rendered via a portal at body level (with fixed positioning) so the scroll
// container clipping inside .settings-scroll can't swallow it.
function CascadeLadder({ rowRect, onLadderEnter, onLadderLeave, setting, layerOverrides, winner, onPromote, onReset, contextLayer, objectOverrides }) {
  if (!rowRect) return null;
  const allDefinedIds = new Set(
    CASCADE_LAYERS
      .filter(l => layerOverrides[l.id] !== undefined && layerOverrides[l.id] !== null)
      .map(l => l.id)
  );

  const LADDER_W = 250;
  const pad = 8;
  // Default position: to the left of the row, vertically centered.
  let left = rowRect.left - LADDER_W - 10;
  if (left < pad) {
    // not enough room on the left — fall back to the right of the row
    left = Math.min(window.innerWidth - LADDER_W - pad, rowRect.right + 10);
  }
  const top = Math.max(pad, Math.min(rowRect.top + rowRect.height / 2, window.innerHeight - pad));

  return ReactDOM.createPortal(
    <div
      className="cascade-ladder"
      style={{ position: "fixed", top, left, transform: "translateY(-50%)" }}
      onMouseEnter={onLadderEnter}
      onMouseLeave={onLadderLeave}
    >
      <div className="ladder-title">Value cascade · {setting.name}</div>
      {CASCADE_LAYERS.filter(l => l.id !== "object").map(layer => {
        const has = allDefinedIds.has(layer.id);
        const v = layerOverrides[layer.id];
        const isWinner = layer.id === winner;
        const isOverridden = has && !isWinner;
        return (
          <div
            key={layer.id}
            className={`ladder-row ${has ? "" : "empty"} ${isWinner ? "winner" : ""} ${isOverridden ? "overridden" : ""}`}
            style={{ "--row-hue": layer.hue }}
          >
            <span className={`l-dot ${has ? "" : "empty"}`}/>
            <span className="l-name">{layer.label}</span>
            <span className={`l-val ${has ? "" : "muted"}`}>
              {has ? formatVal(v, setting.unit) : "—"}
            </span>
          </div>
        );
      })}

      {objectOverrides && objectOverrides.length > 0 && (
        <>
          <div className="ladder-section-title">
            <span className="ladder-section-dot" style={{ background: `hsl(${LAYER_BY_ID.object.hue} 70% 55%)` }}/>
            {objectOverrides.length} object{objectOverrides.length > 1 ? "s" : ""} override
          </div>
          {objectOverrides.map(({ obj, value, filament, isSelected }) => (
            <div
              key={obj.id}
              className={`ladder-row obj-row ${isSelected ? "winner" : ""}`}
              style={{ "--row-hue": LAYER_BY_ID.object.hue }}
            >
              <span className="l-dot" style={{ background: filament?.color || "#888" }}/>
              <span className="l-name" title={obj.name}>{obj.name}</span>
              <span className="l-val">{formatVal(value, setting.unit)}</span>
            </div>
          ))}
        </>
      )}
    </div>,
    document.body
  );
}

function formatVal(v, unit) {
  if (typeof v === "boolean") return v ? "on" : "off";
  if (typeof v === "number") {
    const s = (Math.round(v * 1000) / 1000).toString();
    return unit ? `${s} ${unit}` : s;
  }
  return String(v);
}

// ───────── Setting Row ─────────
function SettingRow({
  setting,
  contextLayer,
  selectedObject,         // present when contextLayer === "object"
  objects,                // ALL objects on the plate, so we can show which override this setting
  filaments,              // for color swatches in the per-object section
  slotMap,                // slot → filamentId
  materialMap,            // materialId → slot
  accountabilityMode,
  userOverrides,          // project-level user edits only
  onSetProjectOverride,
  onResetProjectOverride,
  onSetObjectOverride,
  onResetObjectOverride,
}) {
  // Which objects on the plate override this specific setting? Used for the
  // "objects-override" badge + the per-object section in the hover ladder.
  const objectOverrides = useSPM(() => {
    if (!objects) return [];
    return objects
      .filter(o => o.overrides && o.overrides[setting.id] !== undefined && o.overrides[setting.id] !== null)
      .map(o => {
        const { filament } = window.SLICER_DATA.resolveObjectFilament(o, materialMap, slotMap, filaments);
        return {
          obj: o,
          value: o.overrides[setting.id],
          filament,
          isSelected: selectedObject && o.id === selectedObject.id,
        };
      });
  }, [objects, filaments, materialMap, slotMap, setting.id, selectedObject]);
  // Effective cascade:
  //   - Base: setting.cascade (printer..project, set by profile data)
  //   - + user project overrides
  //   - + selected object's overrides (only on Object tab)
  const layerOverrides = useSPM(() => {
    const out = { ...setting.cascade };
    const user = userOverrides[setting.id] || {};
    Object.entries(user).forEach(([k, v]) => {
      if (v === undefined || v === null) delete out[k]; else out[k] = v;
    });
    if (contextLayer === "object" && selectedObject) {
      const objVal = (selectedObject.overrides || {})[setting.id];
      if (objVal !== undefined && objVal !== null) {
        out.object = objVal;
      }
    }
    return out;
  }, [setting, userOverrides, contextLayer, selectedObject]);

  const { value, layer } = useSPM(() => {
    for (let i = CASCADE_LAYERS.length - 1; i >= 0; i--) {
      const id = CASCADE_LAYERS[i].id;
      if (layerOverrides[id] !== undefined && layerOverrides[id] !== null) {
        return { value: layerOverrides[id], layer: id };
      }
    }
    return { value: setting.default, layer: "printer" };
  }, [layerOverrides]);

  const layerMeta = LAYER_BY_ID[layer];
  const definedLayers = CASCADE_LAYERS.filter(l => layerOverrides[l.id] !== undefined);
  const isOverridden = definedLayers.length > 1;
  const hasProjectAuthored = layerOverrides.project !== undefined;
  const hasObjectAuthored  = layerOverrides.object  !== undefined;
  const conflict = isOverridden && definedLayers[definedLayers.length - 1].id === "object"
    && definedLayers.some(l => l.id === "project" && layerOverrides.project !== layerOverrides.object);

  // Writes go to the layer matching the current tab.
  const handleChange = (newVal) => {
    if (contextLayer === "object" && selectedObject) {
      onSetObjectOverride(setting.id, newVal);
    } else {
      onSetProjectOverride(setting.id, newVal);
    }
  };
  const handleReset = () => {
    if (contextLayer === "object" && selectedObject) {
      onResetObjectOverride(setting.id);
    } else {
      onResetProjectOverride(setting.id);
    }
  };

  // Reset is available whenever the ACTIVE editing layer has any value for this
  // setting — pre-seeded from profile data or added by the user.
  const hasValueAtContext = layerOverrides[contextLayer] !== undefined;

  // ─── Hover handling for the cascade ladder ───
  // Ladder is rendered via portal at body level so it isn't clipped by the
  // scroll container. We open on row hover, close on a short delay so the user
  // can move from the row to the ladder without losing it.
  const [ladderOpen, setLadderOpen] = useSPS(false);
  const [rowRect, setRowRect] = useSPS(null);
  const rowRef = useSPR(null);
  const closeT = useSPR(null);
  const openLadder = () => {
    if (closeT.current) { clearTimeout(closeT.current); closeT.current = null; }
    if (rowRef.current) setRowRect(rowRef.current.getBoundingClientRect());
    setLadderOpen(true);
  };
  const scheduleClose = () => {
    if (closeT.current) clearTimeout(closeT.current);
    closeT.current = setTimeout(() => setLadderOpen(false), 120);
  };
  useSPE(() => () => { if (closeT.current) clearTimeout(closeT.current); }, []);

  const renderControl = () => {
    if (setting.type === "toggle") {
      return (
        <div className="val-toggle-wrap">
          <div className={`val-toggle ${value ? "on" : ""}`} onClick={() => handleChange(!value)}/>
        </div>
      );
    }
    if (setting.type === "select") {
      return (
        <select className="val-select" value={value} onChange={(e) => handleChange(e.target.value)}>
          {(setting.options || []).map(o => <option key={o} value={o}>{o}</option>)}
        </select>
      );
    }
    return (
      <div className="val-wrap">
        <input
          className="val-input"
          type="number"
          value={value}
          step={setting.step || 1}
          min={setting.min}
          max={setting.max}
          onChange={(e) => handleChange(parseFloat(e.target.value))}
        />
        {setting.unit && <span className="val-unit">{setting.unit}</span>}
      </div>
    );
  };

  return (
    <div
      ref={rowRef}
      className={`set-row ${isOverridden ? "overridden" : ""} ${conflict ? "has-conflict" : ""} ${hasObjectAuthored ? "authored-object" : hasProjectAuthored ? "authored-project" : ""}`}
      style={{
        "--row-hue": layerMeta.hue,
        "--proj-hue": LAYER_BY_ID.project.hue,
        "--obj-hue":  LAYER_BY_ID.object.hue,
      }}
      data-setting-id={setting.id}
      onMouseEnter={openLadder}
      onMouseLeave={scheduleClose}
    >
      <div className="set-meta">
        <span className="set-name" title={setting.name}>{setting.name}</span>
        {accountabilityMode === "breadcrumb" && (
          <span className="set-breadcrumb">
            {definedLayers.map((l, i) => (
              <React.Fragment key={l.id}>
                <span
                  className={`crumb ${l.id === layer ? "" : "muted"}`}
                  style={{ "--crumb-hue": l.hue }}
                >{l.short}</span>
                {i < definedLayers.length - 1 && <span className="arrow">›</span>}
              </React.Fragment>
            ))}
          </span>
        )}
        {objectOverrides.length > 0 && contextLayer !== "object" && (
          <span
            className="objs-badge"
            style={{ "--obj-hue": LAYER_BY_ID.object.hue }}
            title={`${objectOverrides.length} object${objectOverrides.length > 1 ? "s" : ""} override this — hover row for details`}
          >
            <svg width="9" height="9" viewBox="0 0 12 12" fill="none" aria-hidden>
              <path d="M2.5 3v3a2 2 0 0 0 2 2H10M10 8L7.5 5.5M10 8L7.5 10.5"
                stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
            </svg>
            <span className="objs-badge-dots">
              {objectOverrides.slice(0, 3).map((o, i) => (
                <span key={o.obj.id} className="objs-badge-dot" style={{ background: o.filament?.color || "#888" }}/>
              ))}
              {objectOverrides.length > 3 && <span className="objs-badge-more">+{objectOverrides.length - 3}</span>}
            </span>
          </span>
        )}
      </div>

      {hasValueAtContext && (
        <button
          className="reset-btn show"
          title={`Reset ${LAYER_BY_ID[contextLayer].label} override (falls back to inherited value)`}
          onClick={(e) => { e.stopPropagation(); handleReset(); }}
        >
          <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
            <path d="M2.5 5a3.5 3.5 0 1 0 1-2.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
            <path d="M2 2v3h3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
          </svg>
        </button>
      )}

      <div className="set-value">{renderControl()}</div>

      {ladderOpen && (
        <CascadeLadder
          setting={setting}
          layerOverrides={layerOverrides}
          winner={layer}
          contextLayer={contextLayer}
          objectOverrides={objectOverrides}
          onPromote={() => handleChange(value)}
          onReset={handleReset}
          rowRect={rowRect}
          onLadderEnter={openLadder}
          onLadderLeave={scheduleClose}
        />
      )}
    </div>
  );
}

// ───────── Slot / Material chips ─────────
// Two small dropdown chips used in the config strip.
//
// SlotChip — represents a physical slot in the printer (ext or AMS:1..4) and
// lets the user re-bind it to any filament in the library.
//
// MaterialChip — represents a logical project material (M1, M2…) and lets the
// user choose which slot it should be printed from.
function SlotChip({ slotId, slotIds, filament, onOpenPicker }) {
  const shortLabel = slotShortLabel(slotId, slotIds);
  const longLabel = slotLongLabel(slotId);
  return (
    <div className="slot-chip-wrap">
      <button
        className={`slot-pill slot-chip-${slotId.replace(/[:]/g,"-")} ${filament ? "" : "empty"}`}
        onClick={() => onOpenPicker(slotId)}
        title={filament
          ? `${longLabel} · ${filament.brand || ""} ${filament.product || filament.label || ""}${filament.colorName ? " (" + filament.colorName + ")" : ""}\nClick to change`
          : `${longLabel} — click to assign filament`}
      >
        <span className="slot-pill-swatch" style={{ background: filament?.color || "transparent" }}/>
        <span className="slot-pill-label">{shortLabel}</span>
        <span className="slot-pill-material">{filament?.material || "—"}</span>
      </button>
    </div>
  );
}

function MaterialChip({ materialId, slotId, filament, useCount, filaments, slotIds = [], slotMap, onPickSlot }) {
  const [open, setOpen] = useSPS(false);
  useSPE(() => {
    if (!open) return;
    const onDoc = (e) => {
      if (!e.target.closest(`.material-menu-${materialId}`) &&
          !e.target.closest(`.material-chip-${materialId}`)) setOpen(false);
    };
    document.addEventListener("click", onDoc);
    return () => document.removeEventListener("click", onDoc);
  }, [open, materialId]);

  return (
    <div className="config-chip-wrap">
      <button
        className={`material-chip material-chip-${materialId}`}
        onClick={() => setOpen(o => !o)}
        title={filament ? `${materialId} → ${slotLongLabel(slotId)} (${filament.label}) · ${useCount} object${useCount !== 1 ? "s" : ""}` : `${materialId} → unmapped`}
      >
        <span className="material-id">{materialId}</span>
        <span className="material-arrow">→</span>
        <span className="material-slot">{slotId ? slotShortLabel(slotId, slotIds) : "—"}</span>
        <span className="fil-swatch" style={{ background: filament?.color || "transparent", border: filament ? "none" : "1px dashed currentColor" }}/>
        <span className="fil-label">{filament ? filament.label : "unmapped"}</span>
        <span className="fil-count">×{useCount}</span>
        <ChevronChip/>
      </button>
      {open && (
        <div className={`printer-picker-menu material-menu material-menu-${materialId}`} onClick={(e) => e.stopPropagation()}>
          <div className="ptpm-title">Route {materialId} to slot…</div>
          {slotIds.map(sid => {
            const fid = (slotMap || {})[sid];
            const f = fid ? filaments.find(x => x.id === fid) : null;
            return (
              <button
                key={sid}
                className={`ptpm-item ptpm-row ${sid === slotId ? "active" : ""}`}
                onClick={() => { onPickSlot(sid); setOpen(false); }}
              >
                <span className="ptpm-name">
                  <span className="ptpm-swatch" style={{ background: f?.color || "transparent", border: f ? "none" : "1px dashed currentColor" }}/>
                  {slotLongLabel(sid)}
                </span>
                <span className="ptpm-detail">{f ? f.label : "empty"}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

// "Slots" row label is the sync button — clicking it pulls the printer's
// current spool loadout into the slicer. Label stays "Slots"; only the icon
// animates: spinning while syncing, briefly flashing a check on success.
// Button is disabled (non-interactive) for the duration of the sync.
function SyncSlotsLabel({ printer, onSync }) {
  const [state, setState] = useSPS("idle"); // idle | syncing | synced
  const timerRef = useSPR(null);
  useSPE(() => () => { if (timerRef.current) clearTimeout(timerRef.current); }, []);

  const handleClick = () => {
    if (state !== "idle") return;
    setState("syncing");
    Promise.resolve(onSync && onSync())
      .catch(() => {})
      .finally(() => {
        // hold the spinner for a beat even if onSync resolves instantly
        timerRef.current = setTimeout(() => {
          setState("synced");
          timerRef.current = setTimeout(() => setState("idle"), 900);
        }, 650);
      });
  };

  const title = state === "syncing"
    ? `Syncing filament loadout from ${printer || "printer"}…`
    : state === "synced"
    ? `Filament loadout synced from ${printer || "printer"}`
    : `Physical loadout — what each slot is spooled with right now.\nClick to sync from ${printer || "the printer"}.`;

  return (
    <button
      className={`config-row-label slots-sync-label state-${state}`}
      onClick={handleClick}
      disabled={state !== "idle"}
      title={title}
      aria-label="Sync filaments from printer"
      aria-busy={state === "syncing"}
    >
      <span className="slots-sync-ico" aria-hidden>
        {state === "synced" ? (
          <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
            <path d="M2.5 6.5l2.2 2.2L9.5 3.8" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round"/>
          </svg>
        ) : (
          <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
            <path d="M10 5.5A4 4 0 0 0 3 3.2" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/>
            <path d="M10 1.5V3.5H8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/>
            <path d="M2 6.5A4 4 0 0 0 9 8.8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/>
            <path d="M2 10.5V8.5H4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/>
          </svg>
        )}
      </span>
      <span className="slots-sync-text">Slots</span>
    </button>
  );
}

// Per-extruder nozzle chip. With one extruder this is the classic "Nozzle"
// chip; with multiple toolheads (Snapmaker U1, Prusa XL) we render one chip
// per toolhead and number them T1, T2, …
function NozzleChip({ index, total, nozzle, onClick }) {
  const label = total > 1 ? `Nozzle T${index + 1}` : "Nozzle";
  return (
    <button
      className="config-chip"
      onClick={onClick}
      style={{ "--chip-hue": LAYER_BY_ID.nozzle.hue }}
      title={`${label} — ${nozzle}\nClick to change`}
    >
      <span className="config-chip-top">
        <span className="chip-dot"/>
        <span className="chip-label">{label}</span>
        <span className="chev"><ChevronChip/></span>
      </span>
      <span className="chip-value" title={nozzle}>{nozzle}</span>
    </button>
  );
}

// ───────── Settings Panel root ─────────
function SettingsPanel({
  contextLayer,
  setContextLayer,
  selectedObject,
  setObjectOverride,
  resetObjectOverride,
  accountabilityMode,
  searchMode,
  userOverrides,
  setUserOverrides,
  objects,                 // all objects on the plate
  filaments,
  // config-strip props
  printer, printerConnStatus = "none", bedPlate, nozzle,
  extruders = 1,
  nozzles,                 // optional per-extruder array; falls back to [nozzle] × extruders
  filamentsInUse,
  materialsInUse,
  slotIds = [],
  slotMap,
  materialMap,
  onOpenSlotPicker,
  onSyncFilaments,
  setMaterialSlot,
  printerPresets,
  onSwapPrinter, onSwapBedPlate, onSwapNozzle, onSwapFilament,
  onEditPrinter,
}) {
  const [query, setQuery] = useSPS("");
  const [activeCat, setActiveCat] = useSPS(CATEGORIES[0].id);
  const [printerMenuOpen, setPrinterMenuOpen] = useSPS(false);
  const [qualityProfile, setQualityProfile] = useSPS("standard");
  const [qualityMenuOpen, setQualityMenuOpen] = useSPS(false);
  const scrollRef = useSPR(null);
  const inputRef = useSPR(null);
  const printerChipRef = useSPR(null);

  // Dismiss printer menu on outside click
  useSPE(() => {
    if (!printerMenuOpen) return;
    const onDoc = (e) => {
      if (!e.target.closest(".printer-picker-menu") && !e.target.closest(".config-chip-printer")) {
        setPrinterMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [printerMenuOpen]);

  // Dismiss quality menu on outside click
  useSPE(() => {
    if (!qualityMenuOpen) return;
    const onDoc = (e) => {
      if (!e.target.closest(".sp-quality-menu") && !e.target.closest(".sp-quality-chip")) {
        setQualityMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [qualityMenuOpen]);

  const objectAvailable = !!selectedObject;

  // Derive the per-extruder nozzle list. Most printers have a single nozzle;
  // the U1 has 4 (one per toolhead), the Prusa XL up to 5. If the plate
  // doesn't store per-extruder strings yet, mirror the default nozzle into
  // each slot so the chips still render.
  const nozzleList = useSPM(() => {
    const n = Math.max(1, extruders | 0);
    if (Array.isArray(nozzles) && nozzles.length) {
      return Array.from({ length: n }, (_, i) => nozzles[i] || nozzle);
    }
    return Array(n).fill(nozzle);
  }, [extruders, nozzles, nozzle]);
  const inlineNozzles = nozzleList.length <= 2;
  // If object becomes unavailable while on object tab, fall back to project.
  useSPE(() => {
    if (contextLayer === "object" && !objectAvailable) setContextLayer("project");
  }, [contextLayer, objectAvailable, setContextLayer]);

  // keyboard shortcut for search focus
  useSPE(() => {
    const handler = (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "f") {
        e.preventDefault();
        inputRef.current?.focus();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const filteredCategories = useSPM(() => {
    if (!query.trim()) return CATEGORIES.map(c => ({ ...c, _settings: c.settings, _matchedAll: true }));

    const q = query.trim();
    return CATEGORIES.map(cat => {
      let matched;
      if (searchMode === "instant") {
        // simple case-insensitive substring on setting name or id
        const lower = q.toLowerCase();
        matched = cat.settings.filter(s =>
          s.name.toLowerCase().includes(lower) || s.id.includes(lower)
        );
      } else if (searchMode === "scoped") {
        // narrows to the active category only
        if (cat.id !== activeCat) matched = [];
        else {
          const lower = q.toLowerCase();
          matched = cat.settings.filter(s =>
            s.name.toLowerCase().includes(lower) || s.id.includes(lower)
          );
        }
      } else { // fuzzy
        matched = cat.settings
          .map(s => ({ s, score: Math.max(fuzzyScore(q, s.name), fuzzyScore(q, s.id) * 0.7) }))
          .filter(x => x.score > 0.25)
          .sort((a, b) => b.score - a.score)
          .map(x => x.s);
      }
      return { ...cat, _settings: matched, _matchedAll: false };
    }).filter(c => c._settings.length > 0);
  }, [query, searchMode, activeCat]);

  // observe scroll position to update activeCat
  useSPE(() => {
    const root = scrollRef.current;
    if (!root) return;
    const headers = Array.from(root.querySelectorAll("[data-cat-id]"));
    const obs = new IntersectionObserver(entries => {
      // pick the topmost-visible
      const visible = entries.filter(e => e.isIntersecting).sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
      if (visible.length > 0) {
        const id = visible[0].target.getAttribute("data-cat-id");
        setActiveCat(id);
      }
    }, { root, rootMargin: "-10% 0px -70% 0px", threshold: [0, 1] });
    headers.forEach(h => obs.observe(h));
    return () => obs.disconnect();
  }, [filteredCategories]);

  const jumpToCat = (catId) => {
    setActiveCat(catId);
    const el = scrollRef.current?.querySelector(`[data-cat-id="${catId}"]`);
    if (el) el.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  // Project-layer writes — go into userOverrides (also support the rare "user"
  // layer write from the cascade ladder, treating it like project).
  const setProjectOverride = useSPC((settingId, value) => {
    setUserOverrides(prev => {
      const cur = { ...(prev[settingId] || {}) };
      cur.project = value;
      return { ...prev, [settingId]: cur };
    });
  }, [setUserOverrides]);

  const resetProjectOverride = useSPC((settingId) => {
    setUserOverrides(prev => {
      const cur = { ...(prev[settingId] || {}) };
      const setting = ALL_SETTINGS.find(s => s.id === settingId);
      const baseHasValue = setting && setting.cascade.project !== undefined && setting.cascade.project !== null;
      if (baseHasValue) {
        // base profile defined a value here — sentinel-clear with null so it
        // doesn't fall through.
        cur.project = null;
      } else {
        // value was a user-added override — just drop the key.
        delete cur.project;
      }
      const next = { ...prev };
      if (Object.keys(cur).length === 0) delete next[settingId];
      else next[settingId] = cur;
      return next;
    });
  }, [setUserOverrides]);

  // Count of settings per category that have an active value at the current
  // editing layer. On Project tab: counts settings with project values (base
  // profile + user additions, minus cleared sentinels). On Object tab: counts
  // settings the selected object has overrides for.
  const counts = useSPM(() => {
    const out = {};
    CATEGORIES.forEach(c => {
      out[c.id] = {
        total: c.settings.length,
        overrides: c.settings.filter(s => {
          if (contextLayer === "object") {
            if (!selectedObject) return false;
            const ov = (selectedObject.overrides || {})[s.id];
            return ov !== undefined && ov !== null;
          }
          // project tab
          const user = (userOverrides[s.id] || {}).project;
          if (user === null) return false;
          if (user !== undefined) return true;
          return s.cascade.project !== undefined && s.cascade.project !== null;
        }).length,
      };
    });
    return out;
  }, [userOverrides, contextLayer, selectedObject]);

  const totalMatches = filteredCategories.reduce((n, c) => n + c._settings.length, 0);

  return (
    <aside className="settings-panel">
      <div className="sp-config">
        <div className="sp-config-row">
          <div className="config-chip-wrap" ref={printerChipRef}>
            <button
              className="config-chip config-chip-printer"
              onClick={() => setPrinterMenuOpen(o => !o)}
              style={{ "--chip-hue": LAYER_BY_ID.printer.hue }}
            >
              <span className="config-chip-top">
                <span className="chip-dot"/>
                <span className="chip-label">Printer</span>
                <span className="chev"><ChevronChip/></span>
              </span>
              <span className="chip-value-row">
                <span className="chip-value" title={printer}>{printer}</span>
                <span
                  className={`conn-indicator conn-${printerConnStatus}`}
                  title={CONN_LABELS[printerConnStatus] || CONN_LABELS.none}
                  aria-label={CONN_LABELS[printerConnStatus] || CONN_LABELS.none}
                />
              </span>
            </button>
            {printerMenuOpen && printerPresets && (
              <div className="printer-picker-menu" onClick={(e) => e.stopPropagation()}>
                <div className="ptpm-title">Assign printer to this plate</div>
                {printerPresets.filter(p => !p.isAddNew).map(preset => (
                  <div
                    key={preset.id}
                    className={`ptpm-item ptpm-row ${preset.name === printer ? "active" : ""}`}
                  >
                    <button
                      className="ptpm-row-main"
                      onClick={() => {
                        onSwapPrinter && onSwapPrinter(preset.id);
                        setPrinterMenuOpen(false);
                      }}
                      title={`Assign ${preset.name} to this plate`}
                    >
                      <span className="ptpm-name">{preset.name}</span>
                      <span className="ptpm-detail">
                        {preset.plateSize[0]}×{preset.plateSize[1]} · {preset.bedPlate}
                      </span>
                    </button>
                    {onEditPrinter && (
                      <button
                        className="ptpm-cog"
                        onClick={(e) => {
                          e.stopPropagation();
                          setPrinterMenuOpen(false);
                          onEditPrinter(preset.id);
                        }}
                        title={`Settings for ${preset.name}`}
                        aria-label={`Settings for ${preset.name}`}
                      >
                        <svg width="13" height="13" viewBox="0 0 14 14" fill="none">
                          <path d="M7 4.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM7 1v1.5M7 11.5V13M3.5 3.5l1 1M9.5 9.5l1 1M1 7h1.5M11.5 7H13M3.5 10.5l1-1M9.5 4.5l1-1" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round"/>
                        </svg>
                      </button>
                    )}
                  </div>
                ))}
                {printerPresets.some(p => p.isAddNew) && (
                  <>
                    <div className="ptpm-divider"/>
                    <button
                      className="ptpm-item ptpm-add"
                      onClick={() => {
                        onSwapPrinter && onSwapPrinter("__new__");
                        setPrinterMenuOpen(false);
                      }}
                    >
                      <span className="ptpm-name">
                        <svg width="11" height="11" viewBox="0 0 14 14" fill="none" style={{verticalAlign:"-1px", marginRight:6}}>
                          <path d="M7 2v10M2 7h10" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round"/>
                        </svg>
                        New printer…
                      </span>
                      <span className="ptpm-detail">⌘N</span>
                    </button>
                  </>
                )}
              </div>
            )}
          </div>
          <button className="config-chip" onClick={onSwapBedPlate} style={{ "--chip-hue": LAYER_BY_ID.printer.hue }}>
            <span className="config-chip-top">
              <span className="chip-dot"/>
              <span className="chip-label">Bed</span>
              <span className="chev"><ChevronChip/></span>
            </span>
            <span className="chip-value" title={bedPlate}>{bedPlate}</span>
          </button>
          {inlineNozzles && nozzleList.map((nz, i) => (
            <NozzleChip
              key={i}
              index={i}
              total={nozzleList.length}
              nozzle={nz}
              onClick={() => onSwapNozzle && onSwapNozzle(i)}
            />
          ))}
        </div>
        {!inlineNozzles && (
          <div className="sp-config-row sp-config-nozzles">
            <span
              className="config-row-label"
              title="Per-toolhead nozzles — one chip per extruder. Click any to change that toolhead's nozzle."
            >Nozzles</span>
            <div className="nozzle-grid">
              {nozzleList.map((nz, i) => (
                <NozzleChip
                  key={i}
                  index={i}
                  total={nozzleList.length}
                  nozzle={nz}
                  onClick={() => onSwapNozzle && onSwapNozzle(i)}
                />
              ))}
            </div>
          </div>
        )}
      <div className="sp-quality" role="row">
        <span className="config-row-label sp-quality-label">Quality</span>
        <div className="sp-quality-wrap">
          {(() => {
            const cur = QUALITY_PROFILES.find(p => p.id === qualityProfile) || QUALITY_PROFILES[0];
            return (
              <button
                className={`sp-quality-chip ${qualityMenuOpen ? "open" : ""}`}
                onClick={() => setQualityMenuOpen(o => !o)}
                aria-haspopup="listbox"
                aria-expanded={qualityMenuOpen}
                title={`${cur.name} · ${cur.layer} mm layer height`}
              >
                <span className="sp-quality-chip-main">
                  <span className="sp-quality-chip-name">{cur.name}</span>
                  <span className="sp-quality-chip-h">{cur.layer} mm</span>
                </span>
                <span className="chev"><ChevronChip/></span>
              </button>
            );
          })()}
          {qualityMenuOpen && (
            <div className="sp-quality-menu" role="listbox" onClick={(e) => e.stopPropagation()}>
              {QUALITY_PROFILES.map(p => (
                <button
                  key={p.id}
                  className={`sp-quality-item ${p.id === qualityProfile ? "active" : ""}`}
                  role="option"
                  aria-selected={p.id === qualityProfile}
                  onClick={() => { setQualityProfile(p.id); setQualityMenuOpen(false); }}
                >
                  <span className="sp-quality-item-name">{p.name}</span>
                  <span className="sp-quality-item-meta">
                    <span className="sp-quality-item-h">{p.layer} mm</span>
                    <span className="sp-quality-item-note">{p.note}</span>
                  </span>
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
        <div className="sp-config-row sp-config-slots">
          <SyncSlotsLabel printer={printer} onSync={onSyncFilaments}/>
          {slotIds.length === 0 ? (
            <span className="dim" style={{ fontSize: 11, fontFamily: "var(--font-mono)" }}>
              no slots — printer has no extruders configured
            </span>
          ) : slotIds.map(slotId => {
            const filamentId = (slotMap || {})[slotId];
            const fil = filamentId ? filaments.find(f => f.id === filamentId) : null;
            return (
              <SlotChip
                key={slotId}
                slotId={slotId}
                slotIds={slotIds}
                filament={fil}
                onOpenPicker={onOpenSlotPicker}
              />
            );
          })}
        </div>
        <div className="sp-config-row sp-config-materials">
          <span className="config-row-label" title="Each material in the project (M1, M2…) is routed to one slot.">Materials</span>
          {(!materialsInUse || materialsInUse.length === 0) ? (
            <span className="dim" style={{ fontSize: 11, fontFamily: "var(--font-mono)" }}>
              none — add an object to assign a material
            </span>
          ) : (
            materialsInUse.map(m => (
              <MaterialChip
                key={m.materialId}
                materialId={m.materialId}
                slotId={m.slotId}
                filament={m.filament}
                useCount={m.useCount}
                filaments={filaments}
                slotIds={slotIds}
                slotMap={slotMap || {}}
                onPickSlot={(slotId) => setMaterialSlot && setMaterialSlot(m.materialId, slotId)}
              />
            ))
          )}
        </div>
      </div>

      <div className="sp-tabs">
        <button
          className={`sp-tab ${contextLayer === "project" ? "active" : ""}`}
          style={{ "--tab-hue": LAYER_BY_ID.project.hue }}
          onClick={() => setContextLayer("project")}
        >
          <span className="sp-tab-dot"/>
          Project
        </button>
        <button
          className={`sp-tab ${contextLayer === "object" ? "active" : ""}`}
          style={{ "--tab-hue": LAYER_BY_ID.object.hue }}
          onClick={() => objectAvailable && setContextLayer("object")}
          disabled={!objectAvailable}
          title={objectAvailable ? `Per-object overrides for ${selectedObject.name}` : "Select an object on the plate to edit per-object overrides"}
        >
          <span className="sp-tab-dot"/>
          Object
        </button>
      </div>

      <div className="search-wrap">
        <div className="search-input">
          <svg className="ico" viewBox="0 0 14 14" fill="none">
            <circle cx="6" cy="6" r="4.2" stroke="currentColor" strokeWidth="1.4"/>
            <path d="M9.2 9.2L12 12" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
          </svg>
          <input
            ref={inputRef}
            type="text"
            placeholder={
              searchMode === "instant" ? "Search 800+ settings…" :
              searchMode === "scoped"  ? `Search within ${LAYER_BY_ID && CATEGORIES.find(c=>c.id===activeCat)?.name}…` :
              "Fuzzy search any setting…"
            }
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          {query && (
            <button className="icon-btn" onClick={() => setQuery("")}>×</button>
          )}
          {!query && <span className="kbd">⌘F</span>}
        </div>
        {query && (
          <div className="dim" style={{ fontSize: 11, fontFamily: "var(--font-mono)" }}>
            {totalMatches} {totalMatches === 1 ? "match" : "matches"} across {filteredCategories.length} {filteredCategories.length === 1 ? "category" : "categories"}
          </div>
        )}
      </div>

      <div className="settings-body">
        <div className="cat-rail">
          {CATEGORIES.map(cat => {
            const hasMatches = !query.trim() || filteredCategories.find(c => c.id === cat.id);
            return (
              <button
                key={cat.id}
                className={`cat-rail-item ${activeCat === cat.id ? "active" : ""}`}
                onClick={() => jumpToCat(cat.id)}
                disabled={!hasMatches}
                style={{ opacity: hasMatches ? 1 : 0.35 }}
              >
                <span className="cat-rail-icon">{cat.icon}</span>
                <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{cat.name}</span>
                <span className="cat-rail-count">
                  {counts[cat.id].overrides > 0
                    ? <span style={{ color: "var(--accent-text)" }}>{counts[cat.id].overrides}/{counts[cat.id].total}</span>
                    : counts[cat.id].total}
                </span>
              </button>
            );
          })}
        </div>

        <div className="settings-scroll" ref={scrollRef}>
          {filteredCategories.length === 0 && (
            <div className="empty-state">
              No settings match <span className="kbd-inline">{query}</span>
              <div style={{ marginTop: 8, fontSize: 11 }}>Try fuzzy mode or broaden your terms.</div>
            </div>
          )}
          {filteredCategories.map(cat => (
            <section key={cat.id} className="cat-group" data-cat-id={cat.id}>
              <header className="cat-header">
                <h4>
                  <span className="cat-rail-icon" style={{ marginRight: 2 }}>{cat.icon}</span>
                  {cat.name}
                  <span className="cat-desc"> · {cat.desc}</span>
                </h4>
                <span className="cat-counts">
                  {cat._settings.length}{!cat._matchedAll && `/${cat.settings.length}`}
                </span>
              </header>
              {cat._settings.map(setting => (
                <SettingRow
                  key={setting.id}
                  setting={setting}
                  contextLayer={contextLayer}
                  selectedObject={contextLayer === "object" ? selectedObject : null}
                  objects={objects}
                  filaments={filaments}
                  slotMap={slotMap}
                  materialMap={materialMap}
                  accountabilityMode={accountabilityMode}
                  userOverrides={userOverrides}
                  onSetProjectOverride={setProjectOverride}
                  onResetProjectOverride={resetProjectOverride}
                  onSetObjectOverride={setObjectOverride}
                  onResetObjectOverride={resetObjectOverride}
                />
              ))}
            </section>
          ))}
        </div>
      </div>
    </aside>
  );
}

window.SettingsPanel = SettingsPanel;
