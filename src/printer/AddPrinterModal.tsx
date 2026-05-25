// Add-printer wizard modal. Two-pane layout: profile gallery on the
// left, spec preview + AMS picker + name input on the right.
//
// Wire-up:
//   - Profiles come from `PrinterCatalogEntry[]` (the bundled catalog).
//   - `existingNames` shadows the user's already-registered instance
//     display names so the validator can flag collisions.
//   - On confirm, fires `onAdd({ printerIdentity, displayName, amsUnits })`
//     and the caller invokes `printer_instance_create` + rebind.
//
// Ported from `docs/design/AddPrinterModal.jsx`. Keyboard: Enter
// confirms when valid, Esc cancels.

import { useEffect, useMemo, useRef, useState } from "react";
import type { PrinterCatalogEntry } from "./printerCommands";

export interface AddPrinterResult {
  printerIdentity: string;
  displayName: string;
  amsUnits: number;
}

/** Compose a name that doesn't collide with `existing`. Appends
 * `" (N)"` with N starting at 2 until a free slot is found.
 * Exported for unit tests; component uses it internally. */
export function makeUniqueName(base: string, existing: readonly string[]): string {
  if (!base) return "";
  if (!existing.includes(base)) return base;
  let n = 2;
  while (existing.includes(`${base} (${n})`)) n++;
  return `${base} (${n})`;
}

/** Build the slot labels for a single-extruder N-AMS-unit topology.
 * Mirrors the Rust backend's `create_instance` topology so the
 * vitest fixture can pin the contract. Returns `[ "Ext", "AMS:1", … ]`
 * for one unit, `[ "Ext", "AMS A:1", … "AMS B:4" ]` for multiple. */
export function amsSlotLabels(amsUnits: number): string[] {
  const labels = ["Ext"];
  for (let unit = 0; unit < amsUnits; unit++) {
    for (let slot = 1; slot <= 4; slot++) {
      const label =
        amsUnits > 1
          ? `AMS ${String.fromCharCode(65 + unit)}:${slot}`
          : `AMS:${slot}`;
      labels.push(label);
    }
  }
  return labels;
}

export interface AddPrinterModalProps {
  catalog: PrinterCatalogEntry[];
  /** Display names already taken by registered instances —
   *  drives the duplicate-name error. */
  existingNames: string[];
  /** Optionally pre-select a profile by identity. */
  initialIdentity?: string | null;
  onAdd: (result: AddPrinterResult) => void;
  onClose: () => void;
}

export function AddPrinterModal({
  catalog,
  existingNames,
  initialIdentity,
  onAdd,
  onClose,
}: AddPrinterModalProps) {
  const [selectedId, setSelectedId] = useState<string | null>(
    initialIdentity ?? catalog[0]?.identity ?? null,
  );
  const [query, setQuery] = useState("");
  const [name, setName] = useState("");
  const [touched, setTouched] = useState(false);
  const [amsUnits, setAmsUnits] = useState(0);
  const nameRef = useRef<HTMLInputElement | null>(null);

  const selected = useMemo(
    () => catalog.find((e) => e.identity === selectedId) ?? null,
    [catalog, selectedId],
  );

  // When the user picks a new profile, reset the AMS count to 1 if
  // the printer supports one (otherwise 0). Cheap UX — the row of
  // tiles defaults to "the typical config" without forcing the user
  // to click.
  useEffect(() => {
    if (!selected) return;
    setAmsUnits(selected.profile.ams_max > 0 ? 1 : 0);
  }, [selectedId, selected]);

  // Auto-fill the name from the selected profile until the user
  // edits it manually.
  useEffect(() => {
    if (selected && !touched) {
      setName(makeUniqueName(selected.profile.model, existingNames));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return catalog;
    return catalog.filter((e) =>
      `${e.profile.brand} ${e.profile.model}`.toLowerCase().includes(q),
    );
  }, [query, catalog]);

  const grouped = useMemo(() => {
    const order: string[] = [];
    const map = new Map<string, PrinterCatalogEntry[]>();
    for (const e of filtered) {
      const brand = e.profile.brand || "Other";
      if (!map.has(brand)) {
        map.set(brand, []);
        order.push(brand);
      }
      map.get(brand)!.push(e);
    }
    return order.map((brand) => ({
      brand,
      items: map.get(brand)!,
    }));
  }, [filtered]);

  const trimmedName = name.trim();
  const nameInUse = trimmedName !== "" && existingNames.includes(trimmedName);
  const canAdd = selected !== null && trimmedName.length > 0 && !nameInUse;

  const handleAdd = (): void => {
    if (!canAdd || !selected) return;
    onAdd({
      printerIdentity: selected.identity,
      displayName: trimmedName,
      amsUnits,
    });
  };

  // Esc-to-close. Enter is wired on the name input directly so it
  // doesn't fire while focus is in the search box.
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
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
          <button
            type="button"
            className="apm-close"
            onClick={onClose}
            aria-label="Close"
            title="Close (Esc)"
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <path
                d="M3 3l8 8M11 3l-8 8"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </header>

        <div className="apm-body">
          <aside className="apm-list" aria-label="Printer profiles">
            <div className="apm-search">
              <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                <circle cx="6" cy="6" r="4" stroke="currentColor" strokeWidth="1.4" />
                <path d="M9 9l3.5 3.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
              </svg>
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search profiles…"
                autoFocus
                aria-label="Search profiles"
              />
              {query && (
                <button
                  type="button"
                  className="apm-search-clear"
                  onClick={() => setQuery("")}
                  aria-label="Clear"
                >
                  <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
                    <path
                      d="M2 2l6 6M8 2l-6 6"
                      stroke="currentColor"
                      strokeWidth="1.4"
                      strokeLinecap="round"
                    />
                  </svg>
                </button>
              )}
            </div>

            <div className="apm-list-scroll">
              {grouped.length === 0 ? (
                <div className="apm-no-results">
                  No profiles match <span className="apm-q">"{query}"</span>
                </div>
              ) : (
                grouped.map((group) => (
                  <div key={group.brand} className="apm-group">
                    <div className="apm-group-label">{group.brand}</div>
                    <div className="apm-cards">
                      {group.items.map((entry) => {
                        const isSel = selectedId === entry.identity;
                        return (
                          <button
                            key={entry.identity}
                            type="button"
                            className={`apm-card ${isSel ? "selected" : ""}`}
                            onClick={() => setSelectedId(entry.identity)}
                          >
                            <div
                              className="apm-card-mark"
                              data-brand={entry.profile.brand}
                            >
                              <span>{entry.profile.brand_short || "?"}</span>
                            </div>
                            <div className="apm-card-info">
                              <div className="apm-card-model">
                                {entry.profile.model}
                              </div>
                              <div className="apm-card-dims">
                                {entry.profile.build_volume.max[0]} ×{" "}
                                {entry.profile.build_volume.max[1]} ×{" "}
                                {entry.profile.build_volume.max[2]} mm
                              </div>
                            </div>
                            <div className="apm-card-check">
                              {isSel && (
                                <svg
                                  width="14"
                                  height="14"
                                  viewBox="0 0 14 14"
                                  fill="none"
                                >
                                  <path
                                    d="M3 7.5l2.8 2.8L11 4.5"
                                    stroke="currentColor"
                                    strokeWidth="1.8"
                                    strokeLinecap="round"
                                    strokeLinejoin="round"
                                  />
                                </svg>
                              )}
                            </div>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                ))
              )}
            </div>
          </aside>

          <section className="apm-detail">
            {selected ? (
              <>
                <div className="apm-preview" data-brand={selected.profile.brand}>
                  <BuildVolumePreview
                    dims={selected.profile.build_volume.max}
                    brand={selected.profile.brand}
                  />
                  <div className="apm-preview-meta">
                    <div
                      className="apm-preview-brand"
                      data-brand={selected.profile.brand}
                    >
                      <span className="apm-preview-mark">
                        {selected.profile.brand_short || "?"}
                      </span>
                      {selected.profile.brand}
                    </div>
                    <div className="apm-preview-model">
                      {selected.profile.model}
                    </div>
                  </div>
                </div>

                <dl className="apm-spec">
                  <div className="apm-spec-row">
                    <dt>Build volume</dt>
                    <dd>
                      {selected.profile.build_volume.max[0]} ×{" "}
                      {selected.profile.build_volume.max[1]} ×{" "}
                      {selected.profile.build_volume.max[2]} mm
                    </dd>
                  </div>
                  <div className="apm-spec-row">
                    <dt>Default nozzle</dt>
                    <dd>
                      {selected.profile.toolheads[0]?.default_nozzle_diameter ?? "—"}
                      mm{" "}
                      {selected.profile.toolheads[0]?.hotend_type ?? ""}
                    </dd>
                  </div>
                  {selected.profile.toolheads.length > 1 && (
                    <div className="apm-spec-row">
                      <dt>Extruders</dt>
                      <dd>{selected.profile.toolheads.length} toolheads</dd>
                    </div>
                  )}
                </dl>

                {selected.profile.ams_max > 0 && (
                  <AmsPicker
                    amsMax={selected.profile.ams_max}
                    amsType={selected.profile.ams_type ?? "AMS"}
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
                      onChange={(e) => {
                        setName(e.target.value);
                        setTouched(true);
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" && canAdd) handleAdd();
                      }}
                      placeholder="e.g. Garage A1"
                    />
                    {touched && name && (
                      <button
                        type="button"
                        className="apm-name-reset"
                        onClick={() => {
                          setTouched(false);
                          if (selected) {
                            setName(
                              makeUniqueName(
                                selected.profile.model,
                                existingNames,
                              ),
                            );
                          }
                        }}
                        title="Reset to profile default"
                      >
                        reset
                      </button>
                    )}
                  </div>
                  {nameInUse ? (
                    <div className="apm-name-hint error">
                      A printer named "{trimmedName}" already exists.
                      Try another.
                    </div>
                  ) : (
                    <div className="apm-name-hint">
                      Shown on plate tabs and in the printer picker. Use
                      whatever helps you tell yours apart.
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
            <button type="button" className="apm-btn" onClick={onClose}>
              Cancel
            </button>
            <button
              type="button"
              className="apm-btn primary"
              onClick={handleAdd}
              disabled={!canAdd}
            >
              Add printer
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}

interface AmsPickerProps {
  amsMax: number;
  amsType: string;
  value: number;
  onChange: (units: number) => void;
}

function AmsPicker({ amsMax, amsType, value, onChange }: AmsPickerProps) {
  const isToggle = amsMax === 1;
  const totalSlots = value * 4 + 1;
  const counterText =
    value === 0
      ? "No AMS"
      : value === 1
        ? `1 × ${amsType} · 4 slots`
        : `${value} × ${amsType} · ${value * 4} slots`;
  return (
    <div className="apm-ams">
      <div className="apm-ams-head">
        <span className="apm-ams-label">{amsType} configuration</span>
        <span className="apm-ams-counter">
          {counterText}
          {value > 0 && (
            <span className="apm-ams-counter-dim">
              {" "}
              (+ ext spool = {totalSlots})
            </span>
          )}
        </span>
      </div>
      {isToggle ? (
        <div className="apm-ams-toggle">
          <button
            type="button"
            className={`apm-ams-tile ${value === 0 ? "active" : ""}`}
            onClick={() => onChange(0)}
          >
            <span className="apm-ams-tile-num">0</span>
            <span className="apm-ams-tile-label">No AMS</span>
          </button>
          <button
            type="button"
            className={`apm-ams-tile ${value === 1 ? "active" : ""}`}
            onClick={() => onChange(1)}
          >
            <span className="apm-ams-tile-num">1</span>
            <span className="apm-ams-tile-label">With {amsType}</span>
            <span className="apm-ams-tile-dots">
              {[0, 1, 2, 3].map((i) => (
                <span key={i} className="apm-ams-tile-dot" />
              ))}
            </span>
          </button>
        </div>
      ) : (
        <div className="apm-ams-row">
          {Array.from({ length: amsMax + 1 }, (_, i) => (
            <button
              key={i}
              type="button"
              className={`apm-ams-tile ${value === i ? "active" : ""}`}
              onClick={() => onChange(i)}
              title={
                i === 0 ? `No ${amsType} installed` : `${i} × ${amsType} (${i * 4} slots)`
              }
            >
              <span className="apm-ams-tile-num">{i}</span>
              <span className="apm-ams-tile-label">
                {i === 0 ? "None" : `${i} unit${i > 1 ? "s" : ""}`}
              </span>
              {i > 0 && (
                <span className="apm-ams-tile-dots">
                  {[0, 1, 2, 3].map((d) => (
                    <span key={d} className="apm-ams-tile-dot" />
                  ))}
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

// Brand color hexes (mirror styles.css [data-brand] tokens but as
// concrete values so the SVG renders correctly even on themes that
// can't resolve color-mix(oklch)).
const BRAND_COLORS: Record<string, string> = {
  "Bambu Lab": "#2F8C5A",
  "Snapmaker": "#3266C8",
  "Prusa": "#D77A2E",
  "Voron": "#7148C9",
  "Creality": "#C84528",
};

interface BuildVolumePreviewProps {
  dims: [number, number, number];
  brand: string;
}

// Tiny isometric wireframe cube. Highlights the bottom face (the
// build plate) with a brand-tinted fill. Dims scale the cube
// proportionally so 250×210 looks different from 256×256.
function BuildVolumePreview({ dims, brand }: BuildVolumePreviewProps) {
  const color = BRAND_COLORS[brand] || "#3F4A5A";
  const [w, d, h] = dims;
  const maxDim = Math.max(w, d, h, 360);
  const scale = 60 / maxDim;
  const wp = w * scale;
  const dp = d * scale;
  const hp = h * scale;

  const cos = Math.cos(Math.PI / 6);
  const sin = Math.sin(Math.PI / 6);
  const cx = 75;
  const cy = 80;
  const p = (x: number, y: number, z: number): [number, number] => {
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

  const corners = [c000, c100, c110, c010, c001, c101, c111, c011];
  const xs = corners.map((c) => c[0]);
  const ys = corners.map((c) => c[1]);
  const offX = 75 - (Math.min(...xs) + Math.max(...xs)) / 2;
  const offY = 75 - (Math.min(...ys) + Math.max(...ys)) / 2;
  const a = (pt: [number, number]): string =>
    `${pt[0] + offX} ${pt[1] + offY}`;

  return (
    <svg
      className="apm-cube-svg"
      viewBox="0 0 150 150"
      fill="none"
      aria-hidden
      style={{ color }}
    >
      <path
        d={`M ${a(c000)} L ${a(c100)} L ${a(c110)} L ${a(c010)} Z`}
        fill={color}
        fillOpacity="0.18"
        stroke={color}
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
      {[0.25, 0.5, 0.75].map((t, i) => {
        const pa = p(wp * t, 0, 0);
        const pb = p(wp * t, dp, 0);
        const pc = p(0, dp * t, 0);
        const pd = p(wp, dp * t, 0);
        return (
          <g key={i} opacity="0.35">
            <path d={`M ${a(pa)} L ${a(pb)}`} stroke={color} strokeWidth="0.6" />
            <path d={`M ${a(pc)} L ${a(pd)}`} stroke={color} strokeWidth="0.6" />
          </g>
        );
      })}
      <path d={`M ${a(c100)} L ${a(c101)}`} stroke={color} strokeWidth="1" strokeDasharray="2 2" opacity="0.55" />
      <path d={`M ${a(c010)} L ${a(c011)}`} stroke={color} strokeWidth="1" strokeDasharray="2 2" opacity="0.55" />
      <path d={`M ${a(c110)} L ${a(c111)}`} stroke={color} strokeWidth="1" strokeDasharray="2 2" opacity="0.55" />
      <path d={`M ${a(c000)} L ${a(c001)}`} stroke={color} strokeWidth="1.4" />
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
