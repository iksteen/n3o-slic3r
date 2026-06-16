// Global hotkey suppression.
//
// App-level keyboard shortcuts (the viewport's Delete/Backspace, the `P`
// preview toggle, the layer-slider arrows, …) must go quiet in two cases:
//   1. a modal is open — a re-entrant refcount that ModalBackdrop pushes on
//      mount and pops on unmount. Stacked modals each push, so hotkeys only
//      come back when the last one closes (a plain boolean would clobber).
//   2. an editable field is focused — a settings input, the group-name editor,
//      a dialog field — so typing (including Backspace) never triggers a canvas
//      shortcut, whether or not that field lives in a modal.
//
// Global handlers gate on `shouldIgnoreHotkey(ev)`. Escape-to-dismiss handlers
// (useModalDismiss / useEscapeKey) deliberately don't — Esc must still close
// the topmost surface while everything else is inhibited.

import { useEffect } from "react";

// Re-entrant inhibit count. >0 ⇒ at least one owner (modal) wants app hotkeys
// off.
let inhibitCount = 0;

/** Push an inhibit; returns the matching release. Call once per owner; the
 *  returned release is idempotent so a double-call can't underflow the count. */
export function inhibitHotkeys(): () => void {
  inhibitCount += 1;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    inhibitCount = Math.max(0, inhibitCount - 1);
  };
}

/** True while at least one owner is inhibiting (a modal is open). */
export function hotkeysInhibited(): boolean {
  return inhibitCount > 0;
}

/** Whether `target` is a text-editing control — input/textarea/select or
 *  contenteditable. */
function isEditableTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el || typeof el.tagName !== "string") return false;
  const tag = el.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return el.isContentEditable === true;
}

/** True when an app-level shortcut should ignore this key event: either a
 *  modal is inhibiting, or the event targets an editable element. */
export function shouldIgnoreHotkey(ev: KeyboardEvent): boolean {
  return hotkeysInhibited() || isEditableTarget(ev.target);
}

/** Inhibit app hotkeys while this component is mounted. ModalBackdrop uses it
 *  so every modal masks hotkeys for free; custom overlays (e.g.
 *  AutosaveRecoveryDialog) can opt in directly. */
export function useHotkeyInhibit(): void {
  useEffect(() => inhibitHotkeys(), []);
}
