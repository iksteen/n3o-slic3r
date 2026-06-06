// PrintersEmptyState.jsx — onboarding when the user has zero printers.
// Full-bleed centered card with primary CTA + supported-brands carousel.

function PrintersEmptyState({ profiles, onAdd }) {
  // Group profiles by brand to show brand chips below the CTA
  const brandsList = React.useMemo(() => {
    const seen = new Set();
    const order = [];
    profiles.forEach(p => {
      if (!seen.has(p.brand)) { seen.add(p.brand); order.push({ brand: p.brand, brandShort: p.brandShort }); }
    });
    return order;
  }, [profiles]);

  return (
    <div className="onboarding-stage">
      {/* Subtle gridded build-plate background */}
      <div className="onboarding-grid" aria-hidden="true"/>

      <div className="onboarding-card">
        <div className="onboarding-mark" aria-hidden="true">
          <svg width="44" height="44" viewBox="0 0 44 44" fill="none">
            {/* Isometric build-plate icon */}
            <path d="M22 8 L36 16 L22 24 L8 16 Z" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" opacity="0.85"/>
            <path d="M22 24 L36 16 L36 28 L22 36 Z" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" opacity="0.5"/>
            <path d="M22 24 L8 16 L8 28 L22 36 Z" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" opacity="0.3"/>
            <path d="M22 36 L22 24" stroke="currentColor" strokeWidth="1.5" opacity="0.6"/>
          </svg>
        </div>

        <h1 className="onboarding-title">Set up your first printer</h1>
        <p className="onboarding-sub">
          Every plate needs a printer. Start with a profile for a popular model,
          then make it yours.
        </p>

        <button className="onboarding-cta" onClick={onAdd} autoFocus>
          <span className="onboarding-cta-plus" aria-hidden="true">
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <path d="M7 2v10M2 7h10" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round"/>
            </svg>
          </span>
          <span className="onboarding-cta-label">Add printer</span>
          <span className="onboarding-cta-kbd">⌘ N</span>
        </button>

        <div className="onboarding-brands">
          <div className="onboarding-brands-label">{profiles.length} profiles</div>
          <div className="onboarding-brands-list">
            {brandsList.map(b => (
              <div key={b.brand} className="onboarding-brand-chip" data-brand={b.brand}>
                <span className="onboarding-brand-mark">{b.brandShort}</span>
                <span>{b.brand}</span>
              </div>
            ))}
          </div>
        </div>

        <div className="onboarding-hint">
          Have a printer not in the list? <a href="#" onClick={(e) => e.preventDefault()}>Import a profile</a>
        </div>
      </div>
    </div>
  );
}

window.PrintersEmptyState = PrintersEmptyState;
