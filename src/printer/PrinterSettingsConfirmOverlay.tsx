/** Modal-blocking confirmation card (discard / delete / AMS-shrink).
 *  Owns the overlay + card + alertdialog/aria wiring + backdrop
 *  dismissal once; callers supply the title, body, and action
 *  buttons. Replaces three byte-near-identical inline copies. */
export function ConfirmOverlay({
  titleId,
  title,
  onBackdrop,
  actions,
  children,
}: {
  titleId: string;
  title: React.ReactNode;
  onBackdrop: () => void;
  /** The footer buttons (Keep editing / Discard / Delete / …). */
  actions: React.ReactNode;
  /** The body copy (and any inline error note). */
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <div className="psm-discard-overlay" onClick={onBackdrop}>
      <div
        className="psm-discard-card"
        onClick={(e) => e.stopPropagation()}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
      >
        <h3 id={titleId} className="psm-discard-title">
          {title}
        </h3>
        {children}
        <div className="psm-discard-actions">{actions}</div>
      </div>
    </div>
  );
}
