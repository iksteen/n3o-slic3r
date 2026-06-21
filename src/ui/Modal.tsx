// Shared modal primitives.
//
// `<ModalBackdrop>` — backdrop click dismisses; inner stops
// propagation. The `.modal-backdrop` CSS class handles
// positioning/blur/centering, so callers don't need inline
// styles.
//
// `<ModalCloseButton>` — the X-icon button modals put in their
// header. Centralizes the SVG so a future icon swap doesn't
// require multi-file edits.

import type { CSSProperties, ReactNode } from "react";
import { useHotkeyInhibit } from "./hotkeyInhibit";

export interface ModalBackdropProps {
  /** Click-outside dismissal handler. Backdrop click + child
   *  click are separated via stopPropagation in the inner wrapper. */
  onDismiss: () => void;
  /** ARIA + class for the inner card. Most modals pass their
   *  modal-specific class (`printer-settings-modal`,
   *  `add-printer-modal`) and the dialog role. */
  cardClassName: string;
  /** Extra class on the `.modal-backdrop` itself — e.g. to raise its
   *  z-index above another modal it was opened from. */
  backdropClassName?: string;
  ariaLabelledBy?: string;
  ariaModal?: boolean;
  role?: "dialog" | "alertdialog";
  cardStyle?: CSSProperties;
  children: ReactNode;
}

export function ModalBackdrop({
  onDismiss,
  cardClassName,
  backdropClassName,
  ariaLabelledBy,
  ariaModal = true,
  role = "dialog",
  cardStyle,
  children,
}: ModalBackdropProps): React.JSX.Element {
  // Mask app hotkeys while any modal is open (re-entrant — stacked modals each
  // hold one inhibit). So Backspace in a dialog field can't delete a scene
  // object behind it, regardless of where focus sits.
  useHotkeyInhibit();
  return (
    <div
      className={`modal-backdrop${backdropClassName ? ` ${backdropClassName}` : ""}`}
      onClick={onDismiss}
    >
      <div
        className={cardClassName}
        onClick={(e) => e.stopPropagation()}
        role={role}
        aria-modal={ariaModal}
        aria-labelledby={ariaLabelledBy}
        style={cardStyle}
      >
        {children}
      </div>
    </div>
  );
}

export interface ModalCloseButtonProps {
  onClick: () => void;
  ariaLabel?: string;
  title?: string;
}

export function ModalCloseButton({
  onClick,
  ariaLabel = "Close",
  title = "Close (Esc)",
}: ModalCloseButtonProps): React.JSX.Element {
  return (
    <button
      className="apm-close"
      onClick={onClick}
      aria-label={ariaLabel}
      title={title}
      type="button"
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
  );
}
