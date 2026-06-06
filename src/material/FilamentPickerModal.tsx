// Three-pane filament picker modal — brand rail → product list →
// color grid (with a Custom swatch that opens the native color
// picker). Port of `docs/dev/design/FilamentPickerModal.jsx`.
//
// Brand and product come from the bundled vendor filament fragments
// (`filament_profile_list`). Color comes from a curated in-app
// palette (`filamentColorPalette.ts`) — Orca/BBS vendor profiles
// don't carry per-color SKUs, so color is a slot property that the
// modal sets alongside the filament identity.
//
// onPick fires with `{ identity, color }` so the caller can route
// each to its existing per-slot writer (setSlotFilament +
// setSlotColor).

import { useEffect, useMemo, useRef, useState } from "react";
import { useModalDismiss } from "../ui/useModalDismiss";
import {
  FILAMENT_COLOR_PALETTE,
  paletteEntryForHex,
  type PaletteColor,
} from "./filamentColorPalette";
import type { FilamentSummary } from "./filamentSummary";

export interface FilamentPickerPick {
  /** Fragment slug — wire form for `SlotBinding.filament_identity`. */
  identity: string;
  /** CSS hex color ("#rrggbb"). */
  color: string;
}

export interface FilamentPickerModalProps {
  /** Header context — short slot label ("AMS:1", "T2", "Ext"). */
  slotId: string;
  /** Filaments to drill into, in display order. */
  filaments: readonly FilamentSummary[];
  /** Seed selection: filament currently loaded in the slot (null
   *  when empty). */
  currentIdentity: string | null;
  /** Seed selection: current spool color, used to either match an
   *  existing palette entry or seed the Custom swatch. */
  currentColor: string | null;
  onPick: (pick: FilamentPickerPick) => void;
  onClose: () => void;
}

// One brand row in the rail. `short` is a 2-3 letter tag for the
// square badge; we derive it from the vendor name initials.
interface BrandGroup {
  brand: string;
  short: string;
  products: FilamentSummary[];
}

function deriveShortTag(brand: string): string {
  // First letter of each whitespace-separated word, max 3 chars,
  // uppercased. "Bambu Lab" → "BL"; "Generic" → "G"; "Polymaker"
  // → "P".
  const initials = brand
    .split(/\s+/)
    .filter(Boolean)
    .map((w) => w[0])
    .join("")
    .toUpperCase()
    .slice(0, 3);
  return initials || brand.slice(0, 3).toUpperCase();
}

function groupByBrand(
  filaments: readonly FilamentSummary[],
): BrandGroup[] {
  const order: string[] = [];
  const map = new Map<string, FilamentSummary[]>();
  for (const f of filaments) {
    if (!map.has(f.vendor)) {
      map.set(f.vendor, []);
      order.push(f.vendor);
    }
    map.get(f.vendor)!.push(f);
  }
  return order.map((brand) => ({
    brand,
    short: deriveShortTag(brand),
    products: map.get(brand)!,
  }));
}

export function FilamentPickerModal({
  slotId,
  filaments,
  currentIdentity,
  currentColor,
  onPick,
  onClose,
}: FilamentPickerModalProps): React.JSX.Element {
  const catalog = useMemo(() => groupByBrand(filaments), [filaments]);
  const materials = useMemo(() => {
    const set = new Set<string>();
    for (const f of filaments) set.add(f.base_type);
    return Array.from(set);
  }, [filaments]);

  // Seed selection from currentIdentity. If the slot's filament
  // belongs to one of the bundled fragments we land on it; otherwise
  // we fall back to (0, 0).
  const seed = useMemo(() => {
    if (currentIdentity) {
      for (let bi = 0; bi < catalog.length; bi++) {
        const idx = catalog[bi].products.findIndex(
          (p) => p.identity === currentIdentity,
        );
        if (idx >= 0) return { brand: bi, product: idx };
      }
    }
    return { brand: 0, product: 0 };
  }, [catalog, currentIdentity]);

  const [query, setQuery] = useState("");
  const [materialFilter, setMaterialFilter] = useState<string | null>(null);
  const [brandIdx, setBrandIdx] = useState(seed.brand);
  const [productIdx, setProductIdx] = useState(seed.product);
  // Seed colorIdx from the slot's current color when it matches the
  // palette; otherwise leave the catalog selection at 0 and let
  // customColor carry the unknown hex.
  const seededPaletteIdx = useMemo(() => {
    const entry = paletteEntryForHex(currentColor);
    if (!entry) return 0;
    return FILAMENT_COLOR_PALETTE.findIndex((c) => c.hex === entry.hex);
  }, [currentColor]);
  const [colorIdx, setColorIdx] = useState(
    seededPaletteIdx >= 0 ? seededPaletteIdx : 0,
  );
  const [customColor, setCustomColor] = useState<string | null>(() => {
    if (!currentColor) return null;
    return paletteEntryForHex(currentColor) ? null : currentColor;
  });

  const customInputRef = useRef<HTMLInputElement | null>(null);
  const searchRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    searchRef.current?.focus();
  }, []);

  useModalDismiss(onClose, { active: true });

  // Color persists across brand / product navigation — palette
  // entries are product-agnostic and the user's color intent
  // shouldn't reset just because they're comparing products. Same
  // for a custom hex.

  const matchesQuery = (text: string): boolean => {
    if (!query.trim()) return true;
    return text.toLowerCase().includes(query.trim().toLowerCase());
  };

  // Brand rail: hide brands whose entire product line is filtered
  // out. Carry `_origIdx` so click handlers can write back into the
  // unfiltered catalog index.
  const visibleBrands = useMemo(() => {
    return catalog
      .map((brand, i) => {
        const products = brand.products.filter(
          (p) =>
            (!materialFilter || p.base_type === materialFilter) &&
            matchesQuery(`${brand.brand} ${p.display_name} ${p.base_type}`),
        );
        return { ...brand, _origIdx: i, _matches: products };
      })
      .filter((b) => b._matches.length > 0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [catalog, query, materialFilter]);

  // Snap brand back into the visible set when filters drop the
  // current pick. Color is intentionally preserved across the snap.
  useEffect(() => {
    if (visibleBrands.length === 0) return;
    const ok = visibleBrands.some((b) => b._origIdx === brandIdx);
    if (!ok) {
      setBrandIdx(visibleBrands[0]._origIdx);
      setProductIdx(0);
    }
  }, [visibleBrands, brandIdx]);

  const currentBrand = catalog[brandIdx] ?? null;
  const currentBrandProducts = useMemo(() => {
    if (!currentBrand) return [] as FilamentSummary[];
    return currentBrand.products.filter(
      (p) =>
        (!materialFilter || p.base_type === materialFilter) &&
        matchesQuery(`${currentBrand.brand} ${p.display_name} ${p.base_type}`),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentBrand, query, materialFilter]);

  useEffect(() => {
    if (productIdx >= currentBrandProducts.length) {
      setProductIdx(0);
    }
  }, [currentBrandProducts.length, productIdx]);

  const currentProduct: FilamentSummary | null =
    currentBrandProducts[productIdx] ?? null;
  const baseColor: PaletteColor | null =
    FILAMENT_COLOR_PALETTE[colorIdx] ?? null;
  const effectiveColor: PaletteColor | null = customColor
    ? {
        name: `Custom ${customColor.toUpperCase()}`,
        hex: customColor,
      }
    : baseColor;

  const handleUse = (): void => {
    if (!currentProduct || !effectiveColor) return;
    onPick({
      identity: currentProduct.identity,
      color: effectiveColor.hex,
    });
  };

  return (
    <div
      className="modal-backdrop fp-modal-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="fp-modal" role="dialog" aria-label="Pick filament">
        <header className="fp-modal-head">
          <div>
            <div className="fp-modal-eyebrow">Slot · {slotId}</div>
            <h2 className="fp-modal-title">Assign filament</h2>
          </div>
          <button
            type="button"
            className="icon-btn fp-modal-close"
            onClick={onClose}
            aria-label="Close"
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <path
                d="M3 3l8 8M11 3l-8 8"
                stroke="currentColor"
                strokeWidth="1.4"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </header>

        <div className="fp-toolbar">
          <div className="fp-search">
            <svg className="ico" viewBox="0 0 14 14" fill="none">
              <circle
                cx="6"
                cy="6"
                r="4.2"
                stroke="currentColor"
                strokeWidth="1.4"
              />
              <path
                d="M9.2 9.2L12 12"
                stroke="currentColor"
                strokeWidth="1.4"
                strokeLinecap="round"
              />
            </svg>
            <input
              ref={searchRef}
              type="text"
              placeholder="Search brand, product, material…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            {query && (
              <button
                type="button"
                className="fp-search-clear"
                onClick={() => setQuery("")}
                title="Clear"
              >
                ×
              </button>
            )}
          </div>
          <div className="fp-material-chips">
            <button
              type="button"
              className={`fp-mat-chip${!materialFilter ? " active" : ""}`}
              onClick={() => setMaterialFilter(null)}
            >
              All
            </button>
            {materials.map((m) => (
              <button
                key={m}
                type="button"
                className={`fp-mat-chip${materialFilter === m ? " active" : ""}`}
                onClick={() =>
                  setMaterialFilter(materialFilter === m ? null : m)
                }
              >
                {m}
              </button>
            ))}
          </div>
        </div>

        <div className="fp-body">
          {/* Brand rail */}
          <ul className="fp-brand-list">
            {visibleBrands.map((b) => (
              <li
                key={b.brand}
                className={`fp-brand-row${b._origIdx === brandIdx ? " active" : ""}`}
                onClick={() => {
                  setBrandIdx(b._origIdx);
                  setProductIdx(0);
                }}
              >
                <span className="fp-brand-short">{b.short}</span>
                <span className="fp-brand-name-wrap">
                  <span className="fp-brand-name">{b.brand}</span>
                  <span className="fp-brand-count">
                    {b._matches.length} product
                    {b._matches.length !== 1 ? "s" : ""}
                  </span>
                </span>
              </li>
            ))}
            {visibleBrands.length === 0 && (
              <li className="fp-empty">No matches</li>
            )}
          </ul>

          {/* Product list */}
          <ul className="fp-product-list">
            {currentBrandProducts.map((p, i) => (
              <li
                key={p.identity}
                className={`fp-product-row${i === productIdx ? " active" : ""}`}
                onClick={() => setProductIdx(i)}
              >
                <span className="fp-product-main">
                  <span className="fp-product-name">{p.display_name}</span>
                  <span className="fp-product-meta">
                    <span className="fp-mat-tag">{p.base_type}</span>
                    <span>
                      {p.nozzle_temp}°C / {p.bed_temp}°C bed
                    </span>
                  </span>
                </span>
              </li>
            ))}
            {currentBrandProducts.length === 0 && (
              <li className="fp-empty">No products match</li>
            )}
          </ul>

          {/* Color grid — shared palette + custom-color swatch. */}
          <div className="fp-color-pane">
            {currentProduct ? (
              <>
                <div className="fp-color-head">
                  <div>
                    <div className="fp-color-product">
                      {currentProduct.display_name}
                    </div>
                    <div className="fp-color-sub">
                      {currentProduct.base_type} · standard palette
                    </div>
                  </div>
                </div>
                <div className="fp-color-grid">
                  {FILAMENT_COLOR_PALETTE.map((c, i) => (
                    <button
                      type="button"
                      key={c.name}
                      className={
                        `fp-color-swatch` +
                        (i === colorIdx && !customColor ? " active" : "") +
                        (c.translucent ? " translucent" : "")
                      }
                      onClick={() => {
                        setColorIdx(i);
                        setCustomColor(null);
                      }}
                      onDoubleClick={() => {
                        setColorIdx(i);
                        setCustomColor(null);
                        setTimeout(handleUse, 0);
                      }}
                      title={c.name}
                    >
                      <span
                        className="fp-color-chip"
                        style={{ background: c.hex }}
                      />
                      <span className="fp-color-name">{c.name}</span>
                    </button>
                  ))}
                  {/* Custom-color swatch. The button itself is the
                      affordance — the native <input type="color"> is
                      pointer-events:none and triggered via
                      `input.click()` so the swatch's hover/active
                      states behave like the palette tiles. */}
                  <button
                    type="button"
                    className={`fp-color-swatch fp-color-swatch-custom${customColor ? " active" : ""}`}
                    onClick={() => customInputRef.current?.click()}
                    title={
                      customColor
                        ? `Custom ${customColor.toUpperCase()} — click to change`
                        : "Pick a custom color"
                    }
                  >
                    <span
                      className={`fp-color-chip${customColor ? "" : " fp-color-chip-rainbow"}`}
                      style={
                        customColor ? { background: customColor } : undefined
                      }
                    >
                      {!customColor && (
                        <svg
                          className="fp-color-chip-plus"
                          width="12"
                          height="12"
                          viewBox="0 0 12 12"
                          fill="none"
                          aria-hidden
                        >
                          <path
                            d="M6 2v8M2 6h8"
                            stroke="currentColor"
                            strokeWidth="1.6"
                            strokeLinecap="round"
                          />
                        </svg>
                      )}
                    </span>
                    <span className="fp-color-name">
                      {customColor ? customColor.toUpperCase() : "Custom…"}
                    </span>
                    <input
                      ref={customInputRef}
                      type="color"
                      className="fp-color-native-input"
                      value={customColor || baseColor?.hex || "#888888"}
                      onChange={(e) => setCustomColor(e.target.value)}
                      tabIndex={-1}
                      aria-label="Pick a custom color"
                    />
                  </button>
                </div>
              </>
            ) : (
              <div className="fp-empty fp-color-empty">Select a product</div>
            )}
          </div>
        </div>

        <footer className="fp-modal-foot">
          <div className="fp-preview">
            {currentProduct && effectiveColor ? (
              <>
                <span
                  className="fp-preview-swatch"
                  style={{ background: effectiveColor.hex }}
                />
                <span className="fp-preview-text">
                  <span className="fp-preview-product">
                    {currentProduct.display_name}
                  </span>
                  <span className="fp-preview-color">
                    {effectiveColor.name} · {currentProduct.base_type} ·{" "}
                    {currentProduct.nozzle_temp}°C
                  </span>
                </span>
              </>
            ) : (
              <span className="dim">No filament selected</span>
            )}
          </div>
          <div className="fp-modal-actions">
            <button type="button" className="apm-btn" onClick={onClose}>
              Cancel
            </button>
            <button
              type="button"
              className="apm-btn primary"
              onClick={handleUse}
              disabled={!currentProduct || !effectiveColor}
            >
              Assign to {slotId}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}
