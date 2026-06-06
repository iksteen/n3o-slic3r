// AddPrinterModal.jsx — modal dialog for creating a new user printer from a
// profile. Two-pane layout: profile gallery on the left, spec preview + name
// input on the right. Enter to confirm, Esc to cancel.

function AddPrinterModal({ profiles, existingNames = [], onAdd, onClose, initialProfileId = null }) {
  const { useState, useEffect, useMemo, useRef } = React;

  const [selectedId, setSelectedId] = useState(initialProfileId || profiles[0]?.id || null);
  const [query, setQuery] = useState("");
  const [name, setName] = useState("");
  const [touched, setTouched] = useState(false);
  const [amsUnits, setAmsUnits] = useState(1); // default to 1 AMS if the profile supports any
  const nameRef = useRef(null);

  const selected = useMemo(
    () => profiles.find(p => p.id === selectedId),
    [profiles, selectedId]
  );

  // Whenever the profile changes, reset the AMS count to a sensible default
  // (1 if the printer supports an AMS at all, 0 otherwise). User can adjust.
  useEffect(() => {
    if (!selected) return;
    setAmsUnits(selected.amsMax > 0 ? 1 : 0);
  }, [selectedId]);

  const makeUniqueName = (base) => {
    if (!base) return "";
    if (!existingNames.includes(base)) return base;
    let n = 2;
    while (existingNames.includes(`${base} (${n})`)) n++;
    return `${base} (${n})`;
  };

  // Auto-fill the name when the user picks a new profile (until they edit it)
  useEffect(() => {
    if (selected && !touched) {
      setName(makeUniqueName(selected.model));
    }
  }, [selectedId]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return profiles;
    return profiles.filter(p =>
      `${p.brand} ${p.model}`.toLowerCase().includes(q)
    );
  }, [query, profiles]);

  const grouped = useMemo(() => {
    const order = [];
    const map = new Map();
    filtered.forEach(p => {
      if (!map.has(p.brand)) { map.set(p.brand, []); order.push(p.brand); }
      map.get(p.brand).push(p);
    });
    return order.map(brand => ({ brand, items: map.get(brand) }));
  }, [filtered]);

  const trimmedName = name.trim();
  const nameInUse = trimmedName && existingNames.includes(trimmedName);
  const canAdd = !!selected && trimmedName.length > 0 && !nameInUse;

  const handleAdd = () => {
    if (!canAdd) return;
    onAdd({ profileId: selected.id, name: trimmedName, amsUnits });
  };

  // ESC to close, Enter (on name input handled inline) to confirm
  useEffect(() => {
    const onKey = (e) => {
      if (e.key === "Escape") { e.stopPropagation(); onClose(); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="add-printer-modal"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-labelledby="apm-title"
      >
        <header className="apm-header">
          <div className="apm-header-text">
            <h2 id="apm-title">Add a printer</h2>
            <p>Pick a profile to base it on. Everything is editable later.</p>
          </div>
          <button className="apm-close" onClick={onClose} aria-label="Close" title="Close (Esc)">
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <path d="M3 3l8 8M11 3l-8 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/>
            </svg>
          </button>
        </header>

        <div className="apm-body">
          <aside className="apm-list" aria-label="Printer profiles">
            <div className="apm-search">
              <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                <circle cx="6" cy="6" r="4" stroke="currentColor" strokeWidth="1.4"/>
                <path d="M9 9l3.5 3.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
              </svg>
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search profiles…"
                autoFocus
                aria-label="Search profiles"
              />
              {query && (
                <button className="apm-search-clear" onClick={() => setQuery("")} aria-label="Clear">
                  <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
                    <path d="M2 2l6 6M8 2l-6 6" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
                  </svg>
                </button>
              )}
            </div>

            <div className="apm-list-scroll">
              {grouped.length === 0 ? (
                <div className="apm-no-results">
                  No profiles match <span className="apm-q">"{query}"</span>
                </div>
              ) : grouped.map(group => (
                <div key={group.brand} className="apm-group">
                  <div className="apm-group-label">{group.brand}</div>
                  <div className="apm-cards">
                    {group.items.map(p => {
                      const isSel = selectedId === p.id;
                      return (
                        <button
                          key={p.id}
                          className={`apm-card ${isSel ? "selected" : ""}`}
                          onClick={() => setSelectedId(p.id)}
                          type="button"
                        >
                          <div className="apm-card-mark" data-brand={p.brand}>
                            <span>{p.brandShort}</span>
                          </div>
                          <div className="apm-card-info">
                            <div className="apm-card-model">{p.model}</div>
                            <div className="apm-card-dims">
                              {p.plateSize[0]} × {p.plateSize[1]} × {p.plateSize[2]} mm
                            </div>
                          </div>
                          <div className="apm-card-check">
                            {isSel && (
                              <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                                <path d="M3 7.5l2.8 2.8L11 4.5" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"/>
                              </svg>
                            )}
                          </div>
                        </button>
                      );
                    })}
                  </div>
                </div>
              ))}
            </div>
          </aside>

          <section className="apm-detail">
            {selected ? (
              <>
                <div className="apm-preview" data-brand={selected.brand}>
                  <BuildVolumePreview dims={selected.plateSize} brand={selected.brand} />
                  <div className="apm-preview-meta">
                    <div className="apm-preview-brand" data-brand={selected.brand}>
                      <span className="apm-preview-mark">{selected.brandShort}</span>
                      {selected.brand}
                    </div>
                    <div className="apm-preview-model">{selected.model}</div>
                  </div>
                </div>

                <dl className="apm-spec">
                  <div className="apm-spec-row">
                    <dt>Build volume</dt>
                    <dd>{selected.plateSize[0]} × {selected.plateSize[1]} × {selected.plateSize[2]} mm</dd>
                  </div>
                  <div className="apm-spec-row">
                    <dt>Default nozzle</dt>
                    <dd>{selected.nozzle}</dd>
                  </div>
                  {(selected.extruders || 1) > 1 && (
                    <div className="apm-spec-row">
                      <dt>Extruders</dt>
                      <dd>{selected.extruders} toolheads</dd>
                    </div>
                  )}
                  {selected.note && (
                    <div className="apm-spec-row">
                      <dt>Notes</dt>
                      <dd className="apm-spec-note">{selected.note}</dd>
                    </div>
                  )}
                </dl>

                {selected.amsMax > 0 && (
                  <AmsPicker
                    amsMax={selected.amsMax}
                    amsType={selected.amsType || "AMS"}
                    value={amsUnits}
                    onChange={setAmsUnits}
                  />
                )}

                <div className="apm-name">
                  <label htmlFor="apm-name-input">Name this printer</label>
                  <div className={`apm-name-input ${nameInUse ? "error" : ""}`}>
                    <input
                      id="apm-name-input"
                      ref={nameRef}
                      value={name}
                      onChange={(e) => { setName(e.target.value); setTouched(true); }}
                      onKeyDown={(e) => { if (e.key === "Enter" && canAdd) handleAdd(); }}
                      placeholder="e.g. Garage A1"
                    />
                    {touched && name && (
                      <button
                        className="apm-name-reset"
                        onClick={() => { setTouched(false); setName(makeUniqueName(selected.model)); }}
                        title="Reset to profile default"
                        type="button"
                      >
                        reset
                      </button>
                    )}
                  </div>
                  {nameInUse ? (
                    <div className="apm-name-hint error">
                      A printer named "{trimmedName}" already exists. Try another.
                    </div>
                  ) : (
                    <div className="apm-name-hint">
                      Shown on plate tabs and in the printer picker. Use whatever helps you tell yours apart.
                    </div>
                  )}
                </div>
              </>
            ) : (
              <div className="apm-no-selection">Pick a profile to continue.</div>
            )}
          </section>
        </div>

        <footer className="apm-footer">
          <span className="apm-keyhint">
            <kbd>↵</kbd> add &nbsp;·&nbsp; <kbd>esc</kbd> cancel
          </span>
          <div className="apm-actions">
            <button className="apm-btn" onClick={onClose} type="button">Cancel</button>
            <button
              className="apm-btn primary"
              onClick={handleAdd}
              disabled={!canAdd}
              type="button"
            >
              Add printer
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}

// AMS configuration picker — shown in the right pane of AddPrinterModal when
// the selected profile supports one or more AMS units.
//
// For amsMax === 1: a binary toggle (no AMS / with AMS).
// For amsMax > 1: a row of selectable "tiles" 0..amsMax so the user can pick
// how many AMS units to install. Each unit contributes 4 slots; the
// running total of slots is shown for clarity.
function AmsPicker({ amsMax, amsType, value, onChange }) {
  const isToggle = amsMax === 1;
  const totalSlots = (value || 0) * 4 + 1; // +1 for the external spool
  return (
    <div className="apm-ams">
      <div className="apm-ams-head">
        <span className="apm-ams-label">{amsType} configuration</span>
        <span className="apm-ams-counter">
          {value === 0
            ? "No AMS"
            : value === 1
              ? `1 × ${amsType} · 4 slots`
              : `${value} × ${amsType} · ${value * 4} slots`}
          {value > 0 && <span className="apm-ams-counter-dim"> (+ ext spool = {totalSlots})</span>}
        </span>
      </div>
      {isToggle ? (
        <div className="apm-ams-toggle">
          <button
            className={`apm-ams-tile ${value === 0 ? "active" : ""}`}
            onClick={() => onChange(0)}
            type="button"
          >
            <span className="apm-ams-tile-num">0</span>
            <span className="apm-ams-tile-label">No AMS</span>
          </button>
          <button
            className={`apm-ams-tile ${value === 1 ? "active" : ""}`}
            onClick={() => onChange(1)}
            type="button"
          >
            <span className="apm-ams-tile-num">1</span>
            <span className="apm-ams-tile-label">With {amsType}</span>
            <span className="apm-ams-tile-dots">
              {[0,1,2,3].map(i => <span key={i} className="apm-ams-tile-dot"/>)}
            </span>
          </button>
        </div>
      ) : (
        <div className="apm-ams-row">
          {Array.from({ length: amsMax + 1 }, (_, i) => (
            <button
              key={i}
              className={`apm-ams-tile ${value === i ? "active" : ""}`}
              onClick={() => onChange(i)}
              type="button"
              title={i === 0 ? `No ${amsType} installed` : `${i} × ${amsType} (${i * 4} slots)`}
            >
              <span className="apm-ams-tile-num">{i}</span>
              <span className="apm-ams-tile-label">
                {i === 0 ? "None" : `${i} unit${i > 1 ? "s" : ""}`}
              </span>
              {i > 0 && (
                <span className="apm-ams-tile-dots">
                  {[0,1,2,3].map(d => <span key={d} className="apm-ams-tile-dot"/>)}
                </span>
              )}
            </button>
          ))}
        </div>
      )}
      <div className="apm-name-hint">
        {value === 0
          ? "Filaments load directly into the extruder via an external spool. You can attach an AMS later from the printer's settings."
          : `Each ${amsType} holds 4 spools and feeds them to the toolhead automatically. You'll route project materials to slots once a plate exists.`}
      </div>
    </div>
  );
}

// Brand color hexes (mirror styles.css [data-brand] tokens but as concrete
// values so SVG renders correctly even in html-to-image captures, which can
// stumble on nested color-mix(oklch).
const BRAND_COLORS = {
  "Bambu Lab": "#2F8C5A",
  "Snapmaker": "#3266C8",
  "Prusa":     "#D77A2E",
  "Voron":     "#7148C9",
  "Creality":  "#C84528",
};

// Tiny isometric wireframe cube. Highlights the bottom face (the build plate)
// with a solid brand-tinted fill. Dims subtly scale the cube width vs depth so
// 250 × 210 looks different from 256 × 256.
function BuildVolumePreview({ dims, brand }) {
  const color = BRAND_COLORS[brand] || "#3F4A5A";
  const [w, d, h] = dims;
  const maxDim = Math.max(w, d, h, 360);
  const scale = 60 / maxDim;
  const wp = w * scale;
  const dp = d * scale;
  const hp = h * scale;

  const cos = Math.cos(Math.PI / 6);
  const sin = Math.sin(Math.PI / 6);

  const cx = 75, cy = 80;
  const p = (x, y, z) => {
    const px = cx + (x - y) * cos;
    const py = cy - z + (x + y) * sin;
    return [px, py];
  };

  const c000 = p(0, 0, 0);
  const c100 = p(wp, 0, 0);
  const c110 = p(wp, dp, 0);
  const c010 = p(0, dp, 0);
  const c001 = p(0, 0, hp);
  const c101 = p(wp, 0, hp);
  const c111 = p(wp, dp, hp);
  const c011 = p(0, dp, hp);

  const xs = [c000, c100, c110, c010, c001, c101, c111, c011].map(c => c[0]);
  const ys = [c000, c100, c110, c010, c001, c101, c111, c011].map(c => c[1]);
  const offX = 75 - (Math.min(...xs) + Math.max(...xs)) / 2;
  const offY = 75 - (Math.min(...ys) + Math.max(...ys)) / 2;
  const a = (pt) => `${pt[0] + offX} ${pt[1] + offY}`;

  return (
    <svg className="apm-cube-svg" viewBox="0 0 150 150" fill="none" aria-hidden="true" style={{ color }}>
      {/* Bottom face — build plate, brand-tinted solid */}
      <path
        d={`M ${a(c000)} L ${a(c100)} L ${a(c110)} L ${a(c010)} Z`}
        fill={color}
        fillOpacity="0.18"
        stroke={color}
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
      {/* Inner grid lines on the build plate (3 strips) — gives it that "plate" feel */}
      {[0.25, 0.5, 0.75].map((t, i) => {
        const pa = p(wp * t, 0, 0);
        const pb = p(wp * t, dp, 0);
        const pc = p(0, dp * t, 0);
        const pd = p(wp, dp * t, 0);
        return (
          <g key={i} opacity="0.35">
            <path d={`M ${a(pa)} L ${a(pb)}`} stroke={color} strokeWidth="0.6"/>
            <path d={`M ${a(pc)} L ${a(pd)}`} stroke={color} strokeWidth="0.6"/>
          </g>
        );
      })}
      {/* Vertical edges (z-axis) — dashed at the back */}
      <path d={`M ${a(c100)} L ${a(c101)}`} stroke={color} strokeWidth="1" strokeDasharray="2 2" opacity="0.55"/>
      <path d={`M ${a(c010)} L ${a(c011)}`} stroke={color} strokeWidth="1" strokeDasharray="2 2" opacity="0.55"/>
      <path d={`M ${a(c110)} L ${a(c111)}`} stroke={color} strokeWidth="1" strokeDasharray="2 2" opacity="0.55"/>
      {/* Front vertical — solid */}
      <path d={`M ${a(c000)} L ${a(c001)}`} stroke={color} strokeWidth="1.4"/>
      {/* Top face outline */}
      <path
        d={`M ${a(c001)} L ${a(c101)} L ${a(c111)} L ${a(c011)} Z`}
        stroke={color}
        strokeWidth="1"
        strokeLinejoin="round"
        opacity="0.55"
      />
    </svg>
  );
}

window.AddPrinterModal = AddPrinterModal;
window.AmsPicker = AmsPicker;
