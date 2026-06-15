// Position a fixed-position popover anchored to a trigger: clamped to
// the viewport horizontally, placed below the trigger, and flipped above
// it when it would overflow the bottom edge. Shared by the object-bar
// pickers (material + send-to-plate).

import type { CSSProperties } from "react";

const PAD = 8;

export function popoverPosition(
  anchorRect: DOMRect,
  menuWidth: number,
  estHeight: number,
): CSSProperties {
  return {
    position: "fixed",
    left: Math.max(
      PAD,
      Math.min(anchorRect.left, window.innerWidth - menuWidth - PAD),
    ),
    top:
      anchorRect.bottom + estHeight > window.innerHeight - PAD
        ? Math.max(PAD, anchorRect.top - estHeight - 4)
        : anchorRect.bottom + 4,
  };
}
