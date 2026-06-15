// Shared dismiss handler for click-outside popovers (chips, dropdowns,
// floating pickers). Closes on a mousedown outside `ref` or on Escape.
//
// Mirrors useModalDismiss (Esc-only, for modals) for the popover case
// where an outside click should also dismiss. Every picker had its own
// document-mousedown effect with subtle drift — some handled Esc, some
// didn't; this gives them all one contract:
//
//   usePopoverDismiss(ref, onDismiss, active)
//
// `active` gates the listeners: pass the popover's `open` flag so the
// effect only binds while open (components that mount only when open can
// omit it — it defaults to true). `onDismiss` is read through a ref, so
// an inline arrow stays fresh without re-subscribing the listeners.

import { useEffect, useRef, type RefObject } from "react";

export function usePopoverDismiss<T extends HTMLElement>(
  ref: RefObject<T | null>,
  onDismiss: () => void,
  active = true,
): void {
  const onDismissRef = useRef(onDismiss);
  onDismissRef.current = onDismiss;
  useEffect(() => {
    if (!active) return;
    const onDoc = (e: MouseEvent): void => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        onDismissRef.current();
      }
    };
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") onDismissRef.current();
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [active, ref]);
}
