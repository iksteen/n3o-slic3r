// Empty-state onboarding shown when the user has zero registered
// `PrinterInstance`s. Mounted by App.tsx in place of the normal
// workspace until the user adds their first printer (which the CTA
// opens `AddPrinterModal` for).
//
// Ported from `docs/dev/design/PrintersEmptyState.jsx` — same layout,
// React 19 + TypeScript shape, brand chips derived from the
// catalog entries.

import { useMemo } from "react";
import type { PrinterCatalogEntry } from "./printerCommands";

export interface PrintersEmptyStateProps {
  /** Bundled printer profiles the wizard offers — used here just to
   *  surface the count + brand chips below the CTA. The actual
   *  list lives in `AddPrinterModal`. */
  catalog: PrinterCatalogEntry[];
  /** Open the add-printer modal. */
  onAdd: () => void;
}

interface BrandChip {
  brand: string;
  brandShort: string;
}

export function PrintersEmptyState({ catalog, onAdd }: PrintersEmptyStateProps) {
  // One chip per unique brand, in first-seen order. Profiles without
  // a brand string (legacy) get skipped — they'd render as a blank
  // chip and just confuse the layout.
  const brandsList = useMemo<BrandChip[]>(() => {
    const seen = new Set<string>();
    const out: BrandChip[] = [];
    for (const entry of catalog) {
      const brand = entry.profile.brand;
      if (!brand || seen.has(brand)) continue;
      seen.add(brand);
      out.push({ brand, brandShort: entry.profile.brand_short || brand[0] });
    }
    return out;
  }, [catalog]);

  return (
    <div className="onboarding-stage">
      <div className="onboarding-grid" aria-hidden />

      <div className="onboarding-card">
        <div className="onboarding-mark" aria-hidden>
          <svg width="44" height="44" viewBox="0 0 44 44" fill="none">
            <path d="M22 8 L36 16 L22 24 L8 16 Z" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" opacity="0.85" />
            <path d="M22 24 L36 16 L36 28 L22 36 Z" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" opacity="0.5" />
            <path d="M22 24 L8 16 L8 28 L22 36 Z" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" opacity="0.3" />
            <path d="M22 36 L22 24" stroke="currentColor" strokeWidth="1.5" opacity="0.6" />
          </svg>
        </div>

        <h1 className="onboarding-title">Set up your first printer</h1>
        <p className="onboarding-sub">
          Every plate needs a printer. Start with a profile for a popular
          model, then make it yours.
        </p>

        <button
          type="button"
          className="onboarding-cta"
          onClick={onAdd}
          autoFocus
        >
          <span className="onboarding-cta-plus" aria-hidden>
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <path d="M7 2v10M2 7h10" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
            </svg>
          </span>
          <span className="onboarding-cta-label">Add printer</span>
        </button>

        <div className="onboarding-brands">
          <div className="onboarding-brands-label">
            {catalog.length} profile{catalog.length === 1 ? "" : "s"}
          </div>
          <div className="onboarding-brands-list">
            {brandsList.map((b) => (
              <div
                key={b.brand}
                className="onboarding-brand-chip"
                data-brand={b.brand}
              >
                <span className="onboarding-brand-mark">{b.brandShort}</span>
                <span>{b.brand}</span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
