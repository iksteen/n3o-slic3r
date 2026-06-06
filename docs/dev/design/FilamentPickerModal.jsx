// FilamentPickerModal.jsx — three-pane filament picker: brand list → product
// list → color grid. The user drills brand → product → color and clicks Use.
//
// Returns to caller via onPick({ id, label, brand, product, material, color,
// colorName, nozzleTemp, bedTemp }). The id is a stable slug derived from
// brand+product+color so picking the same filament twice resolves to the
// same registry entry.

const { useState: useFPS, useMemo: useFPM, useRef: useFPR, useEffect: useFPE } = React;

function FilamentPickerModal({
  slotId,             // which slot the filament will load into (header context)
  currentFilamentId,  // filament currently in the slot — used to seed selection
  onPick,
  onClose,
}) {
  const catalog = window.FILAMENT_CATALOG;
  const materials = window.CATALOG_MATERIALS;
  // Color is decoupled from product — every product can be ordered in every
  // color from the shared standard palette.
  const palette = window.STANDARD_PALETTE || [];

  const [query, setQuery] = useFPS("");
  const [materialFilter, setMaterialFilter] = useFPS(null); // string | null
  const [brandIdx, setBrandIdx] = useFPS(0);
  const [productIdx, setProductIdx] = useFPS(0);
  const [colorIdx, setColorIdx] = useFPS(0);
  // Custom-color override. When set, supersedes the catalog color the user
  // picked from the grid (colorIdx still tracks the swatch they're sitting on,
  // so unsetting custom returns them to the same catalog color).
  const [customColor, setCustomColor] = useFPS(null); // "#rrggbb" | null
  const customInputRef = useFPR(null);
  const searchRef = useFPR(null);

  useFPE(() => { searchRef.current?.focus(); }, []);

  // Escape to close
  useFPE(() => {
    const onKey = (e) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Filter products by material + query
  const matchesQuery = (text) => {
    if (!query.trim()) return true;
    return text.toLowerCase().includes(query.trim().toLowerCase());
  };

  // Apply filters to brand list: show brand iff it has at least one product
  // surviving the material+query filters.
  const visibleBrands = useFPM(() => {
    return catalog.map((brand, i) => {
      const products = brand.products.filter(p =>
        (!materialFilter || p.material === materialFilter) &&
        (matchesQuery(`${brand.brand} ${p.name} ${p.material}`))
      );
      return { ...brand, _origIdx: i, _matches: products };
    }).filter(b => b._matches.length > 0);
  }, [catalog, query, materialFilter]);

  // Snap brand/product indices when filters change. Color selection is
  // preserved across brand/product switches — the palette is shared, so the
  // user's chosen color stays put when they shop other filaments.
  useFPE(() => {
    if (visibleBrands.length === 0) return;
    const ok = visibleBrands.some(b => b._origIdx === brandIdx);
    if (!ok) {
      setBrandIdx(visibleBrands[0]._origIdx);
      setProductIdx(0);
    }
  }, [visibleBrands, brandIdx]);

  const currentBrand = catalog[brandIdx];
  const currentBrandProducts = useFPM(() => {
    if (!currentBrand) return [];
    return currentBrand.products.filter(p =>
      (!materialFilter || p.material === materialFilter) &&
      (matchesQuery(`${currentBrand.brand} ${p.name} ${p.material}`))
    );
  }, [currentBrand, query, materialFilter]);

  useFPE(() => {
    if (productIdx >= currentBrandProducts.length) {
      setProductIdx(0);
    }
  }, [currentBrandProducts.length, productIdx]);

  // (Color selection is intentionally preserved across brand/product changes —
  // the palette is shared and a custom hex is still meaningful for any product.)

  const currentProduct = currentBrandProducts[productIdx];
  const baseColor = palette[colorIdx];
  // Effective color: the custom hex if set, otherwise the catalog color the
  // user has highlighted.
  const currentColor = customColor
    ? { name: `Custom ${customColor.toUpperCase()}`, hex: customColor, translucent: false, _custom: true }
    : baseColor;

  const handleUse = () => {
    if (!currentBrand || !currentProduct || !currentColor) return;
    // For custom colors we still slug against the catalog name so each custom
    // hex gets its own stable id (and round-trips to the same filament entry).
    const slugSeed = currentColor._custom
      ? `${currentBrand.brand}-${currentProduct.name}-custom-${currentColor.hex.replace("#","")}`
      : null;
    const filament = {
      id: slugSeed
        ? window.filamentSlug(currentBrand.brand, currentProduct.name, `custom-${currentColor.hex.replace("#","")}`)
        : window.filamentSlug(currentBrand.brand, currentProduct.name, currentColor.name),
      brand: currentBrand.brand,
      product: currentProduct.name,
      material: currentProduct.material,
      label: `${currentBrand.brand} ${currentProduct.name}`,
      colorName: currentColor.name,
      color: currentColor.hex,
      translucent: !!currentColor.translucent,
      nozzleTemp: currentProduct.nozzleTemp,
      bedTemp: currentProduct.bedTemp,
      custom: !!currentColor._custom,
    };
    onPick(filament);
  };

  return (
    <div className="modal-backdrop fp-modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="fp-modal" role="dialog" aria-label="Pick filament">
        <header className="fp-modal-head">
          <div>
            <div className="fp-modal-eyebrow">Slot · {slotId}</div>
            <h2 className="fp-modal-title">Assign filament</h2>
          </div>
          <button className="icon-btn fp-modal-close" onClick={onClose} aria-label="Close">
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <path d="M3 3l8 8M11 3l-8 8" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
            </svg>
          </button>
        </header>

        <div className="fp-toolbar">
          <div className="fp-search">
            <svg className="ico" viewBox="0 0 14 14" fill="none">
              <circle cx="6" cy="6" r="4.2" stroke="currentColor" strokeWidth="1.4"/>
              <path d="M9.2 9.2L12 12" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/>
            </svg>
            <input
              ref={searchRef}
              type="text"
              placeholder="Search brand, product, material…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            {query && (
              <button className="fp-search-clear" onClick={() => setQuery("")} title="Clear">×</button>
            )}
          </div>
          <div className="fp-material-chips">
            <button
              className={`fp-mat-chip ${!materialFilter ? "active" : ""}`}
              onClick={() => setMaterialFilter(null)}
            >All</button>
            {materials.map(m => (
              <button
                key={m}
                className={`fp-mat-chip ${materialFilter === m ? "active" : ""}`}
                onClick={() => setMaterialFilter(materialFilter === m ? null : m)}
              >{m}</button>
            ))}
          </div>
        </div>

        <div className="fp-body">
          {/* Brand rail */}
          <ul className="fp-brand-list">
            {visibleBrands.map(b => (
              <li
                key={b.brand}
                className={`fp-brand-row ${b._origIdx === brandIdx ? "active" : ""}`}
                onClick={() => { setBrandIdx(b._origIdx); setProductIdx(0); }}
              >
                <span className="fp-brand-short">{b.short}</span>
                <span className="fp-brand-name-wrap">
                  <span className="fp-brand-name">{b.brand}</span>
                  <span className="fp-brand-count">{b._matches.length} product{b._matches.length !== 1 ? "s" : ""}</span>
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
                key={p.name}
                className={`fp-product-row ${i === productIdx ? "active" : ""}`}
                onClick={() => { setProductIdx(i); }}
              >
                <span className="fp-product-main">
                  <span className="fp-product-name">{p.name}</span>
                  <span className="fp-product-meta">
                    <span className="fp-mat-tag">{p.material}</span>
                    <span>{p.nozzleTemp}°C / {p.bedTemp}°C bed</span>
                  </span>
                </span>
              </li>
            ))}
            {currentBrandProducts.length === 0 && (
              <li className="fp-empty">No products match</li>
            )}
          </ul>

          {/* Color grid */}
          <div className="fp-color-pane">
            {currentProduct ? (
              <>
                <div className="fp-color-head">
                  <div>
                    <div className="fp-color-product">{currentBrand.brand} {currentProduct.name}</div>
                    <div className="fp-color-sub">{currentProduct.material} · standard palette</div>
                  </div>
                </div>
                <div className="fp-color-grid">
                  {palette.map((c, i) => (
                    <button
                      key={c.name}
                      className={`fp-color-swatch ${i === colorIdx && !customColor ? "active" : ""} ${c.translucent ? "translucent" : ""}`}
                      onClick={() => { setColorIdx(i); setCustomColor(null); }}
                      onDoubleClick={() => { setColorIdx(i); setCustomColor(null); setTimeout(handleUse, 0); }}
                      title={c.name}
                    >
                      <span className="fp-color-chip" style={{ background: c.hex }}/>
                      <span className="fp-color-name">{c.name}</span>
                    </button>
                  ))}
                  {/* Custom color — opens native color picker. When a custom
                      color is active, the chip shows that color; click again
                      to re-open the picker, or pick a catalog swatch to drop
                      back to a stock color. */}
                  <button
                    className={`fp-color-swatch fp-color-swatch-custom ${customColor ? "active" : ""}`}
                    onClick={() => customInputRef.current?.click()}
                    title={customColor ? `Custom ${customColor.toUpperCase()} — click to change` : "Pick a custom color"}
                  >
                    <span
                      className={`fp-color-chip ${customColor ? "" : "fp-color-chip-rainbow"}`}
                      style={customColor ? { background: customColor } : undefined}
                    >
                      {!customColor && (
                        <svg className="fp-color-chip-plus" width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden>
                          <path d="M6 2v8M2 6h8" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round"/>
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
            {currentProduct && currentColor ? (
              <>
                <span className="fp-preview-swatch" style={{ background: currentColor.hex }}/>
                <span className="fp-preview-text">
                  <span className="fp-preview-product">{currentBrand.brand} {currentProduct.name}</span>
                  <span className="fp-preview-color">{currentColor.name} · {currentProduct.material} · {currentProduct.nozzleTemp}°C</span>
                </span>
              </>
            ) : (
              <span className="dim">No filament selected</span>
            )}
          </div>
          <div className="fp-modal-actions">
            <button className="apm-btn" onClick={onClose}>Cancel</button>
            <button
              className="apm-btn primary"
              onClick={handleUse}
              disabled={!currentBrand || !currentProduct || !currentColor}
            >
              Assign to {slotId}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}

window.FilamentPickerModal = FilamentPickerModal;
