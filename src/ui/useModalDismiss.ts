// Shared Esc-to-dismiss handler for modals.
//
// Every modal in the codebase had its own window-keydown effect
// with subtle drift — some called stopPropagation, some didn't;
// the discard-confirmation flow funnels through requestClose
// rather than onClose directly. This hook gives them all the
// same contract:
//
//   useModalDismiss(handler, { active })
//
// `handler` runs on Escape (and only on Escape) while `active`
// is true. The effect rebinds whenever `handler` or `active`
// changes, so closures-over-state stay fresh — e.g. a modal
// that decides between "show overlay" and "close immediately"
// based on dirty-state can pass an inline arrow without
// stale-closure surprises.

import { useEffect } from "react";

export interface UseModalDismissOptions {
  /** When `false`, the listener is uninstalled — useful for
   *  modals whose parent decides visibility via a state flag. */
  active: boolean;
}

export function useModalDismiss(
  onDismiss: () => void,
  { active }: UseModalDismissOptions,
): void {
  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key !== "Escape") return;
      // stopPropagation prevents a parent window-keydown listener
      // (e.g. a stacked modal) from also acting. Single Esc =
      // single dismissal; whoever is topmost-active wins.
      e.stopPropagation();
      onDismiss();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, onDismiss]);
}
