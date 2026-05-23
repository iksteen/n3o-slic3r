// Setting label tooltip (PR-4-11) — FR-UI-6.
//
// Hover the row's label to see libslic3r's `tooltip` text + an
// optional "💡 why this matters" annotation. The annotations are
// authored in src/settings/annotations/data.ts (PR-4-12); this
// component is type-safe over the data shape but doesn't depend
// on a populated map (annotations beyond ~30 are a cut candidate).

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

export interface SettingTooltipProps {
  /** libslic3r's tooltip text — from `OptionSummary.tooltip`. */
  libslic3rTooltip: string | null;
  /** Optional "why this matters" annotation authored separately
   *  from libslic3r's text. */
  whyThisMatters: string | null;
  /** Anchor element. */
  anchor: HTMLElement | null;
  open: boolean;
  onMouseEnter?: () => void;
  onMouseLeave?: () => void;
}

const TOOLTIP_MAX_WIDTH = 320;
const VIEWPORT_PAD = 8;

export function SettingTooltip({
  libslic3rTooltip,
  whyThisMatters,
  anchor,
  open,
  onMouseEnter,
  onMouseLeave,
}: SettingTooltipProps) {
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const tooltipRef = useRef<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    if (!open || !anchor) {
      setPos(null);
      return;
    }
    const rect = anchor.getBoundingClientRect();
    // Position below the label by default; flip above when the
    // anchor is near the viewport bottom.
    const top =
      rect.bottom + 4 + 100 > window.innerHeight
        ? rect.top - 4
        : rect.bottom + 4;
    const left = Math.min(
      rect.left,
      window.innerWidth - TOOLTIP_MAX_WIDTH - VIEWPORT_PAD,
    );
    setPos({ top: Math.max(VIEWPORT_PAD, top), left: Math.max(VIEWPORT_PAD, left) });
  }, [open, anchor]);

  if (!open || !pos) return null;
  if (!libslic3rTooltip && !whyThisMatters) return null;

  return createPortal(
    <div
      ref={tooltipRef}
      role="tooltip"
      className="setting-tooltip"
      style={{ position: "fixed", top: pos.top, left: pos.left, maxWidth: TOOLTIP_MAX_WIDTH }}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
    >
      {libslic3rTooltip && (
        <div className="tooltip-libslic3r">{libslic3rTooltip}</div>
      )}
      {whyThisMatters && (
        <div className="tooltip-why">
          <span className="tooltip-why-icon" aria-hidden style={{ marginRight: 4 }}>
            💡
          </span>
          <span className="tooltip-why-heading">tip</span>
          {whyThisMatters}
        </div>
      )}
    </div>,
    document.body,
  );
}

/** Lifecycle hook owning the hover open/close. The label invokes
 *  `openAt(el)` on `mouseenter`; the wrapper or label invokes
 *  `scheduleClose()` on `mouseleave`. */
export function useTooltipHover() {
  const [open, setOpen] = useState(false);
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const openAt = (el: HTMLElement) => {
    if (closeTimer.current) {
      clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
    setAnchor(el);
    setOpen(true);
  };
  const scheduleClose = () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
    closeTimer.current = setTimeout(() => setOpen(false), 200);
  };

  useEffect(() => () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
  }, []);

  return { open, anchor, openAt, scheduleClose };
}
