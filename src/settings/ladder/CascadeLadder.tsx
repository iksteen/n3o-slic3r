// Hover cascade ladder (PR-4-8) — FR-CAS-7.
//
// Renders every cascade layer for the hovered setting in a portal
// at body level so the SettingsPanel's overflow scroll doesn't
// clip the popover. Pattern lifted from
// docs/dev/design/SettingsPanel.jsx:39-107 (CascadeLadder function):
//
// - Auto-position left of the row, fall back right if not enough
//   space.
// - 120 ms close delay so the cursor can travel from row to
//   ladder without losing it (matches mockup line 217).
// - Per-layer row with `.l-dot` + `.l-name` + `.l-val`; winner
//   gets `winner` modifier; defined-but-losing layers get
//   `overridden`.
// - Per-object section appended when the hovered setting has
//   object-tier overrides anywhere on the plate (PR-4-9 will fill
//   the per-object list; PR-4-8 ships the section header + an
//   empty body when no objects override).

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  LAYER_HUE,
  type CascadeLayer,
} from "../layers";

/** Per-layer value snapshot for the ladder. `value === null` =
 *  layer didn't define a value (rendered as em-dash). */
export type LadderLayer = {
  id: CascadeLayer;
  label: string;
  value: string | null;
};

/** The cascade layers in priority order (low → high), matching
 *  docs/dev/design/data.jsx. The `object` tier is appended only when the
 *  panel is in object scope (see `objectTier`); in project scope it stays
 *  elided and the objects that override are listed in the per-object
 *  section below instead. */
const LAYER_ORDER: ReadonlyArray<{ id: CascadeLayer; label: string }> = [
  { id: "default", label: "Defaults" },
  { id: "printer", label: "Printer" },
  { id: "build_plate", label: "Build plate" },
  { id: "nozzle", label: "Nozzle" },
  { id: "filament", label: "Filament" },
  { id: "user", label: "Profile" },
  { id: "project", label: "Project" },
];

export interface ObjectOverrideEntry {
  /** Object id (or any stable identifier the host uses). */
  id: number | string;
  /** Display name shown next to the swatch. */
  name: string;
  /** Filament color for the swatch dot. */
  color?: string | null;
  /** The overridden value (formatted). */
  value: string;
}

export interface CascadeLadderProps {
  /** Setting key — drives the popover header. */
  settingKey: string;
  settingLabel: string;
  /** Per-layer value snapshot. Missing layers render as em-dash. */
  layers: ReadonlyMap<CascadeLayer, string | null>;
  /** The winning layer (for the highlight). */
  winningLayer: CascadeLayer;
  /** Anchor element — the popover positions relative to its rect. */
  anchor: HTMLElement | null;
  /** Whether the popover should be visible. The caller manages
   *  hover open/close via the schedule helpers below. */
  open: boolean;
  /** Callback fired when the mouse enters the ladder body — used
   *  by the parent's close-schedule to cancel the pending close so
   *  the cursor can travel to the ladder without losing it. */
  onMouseEnter?: () => void;
  /** Callback fired when the mouse leaves the ladder body —
   *  re-schedules the close. */
  onMouseLeave?: () => void;
  /** When the override is from project / object / user tier, the
   *  authored cascade would otherwise resolve to this value. Shown
   *  as a `cascade fallback` separator + value below the main
   *  layer list. `null` when no override is active. */
  cascadeFallback?: string | null;
  /** Objects on the plate that override this setting. Empty when
   *  no per-object overrides (or PR-4-9 hasn't populated yet). */
  objectOverrides?: readonly ObjectOverrideEntry[];
  /** When the panel is in object scope, the selected object joins the
   *  ladder as its top (highest-priority) tier. `label` is the row name
   *  (the object's name); the value comes from the `object` entry of the
   *  `layers` map. `null` in project scope — the object tier stays out of
   *  the ladder and overriding objects are listed below instead. */
  objectTier?: { label: string } | null;
  /** Upstream libslic3r tooltip describing what the setting does.
   *  Rendered as a small dim paragraph above the cascade title.
   *  `null` when the schema has no tooltip text. */
  description?: string | null;
  /** "Why this matters" annotation from `src/settings/annotations`.
   *  When present, rendered as a small `tip:` line under the
   *  description. */
  whyThisMatters?: string | null;
}

const LADDER_WIDTH = 360;
const VIEWPORT_PAD = 8;

export function CascadeLadder({
  settingKey,
  settingLabel,
  layers,
  winningLayer,
  anchor,
  open,
  onMouseEnter,
  onMouseLeave,
  cascadeFallback = null,
  objectOverrides = [],
  objectTier = null,
  description = null,
  whyThisMatters = null,
}: CascadeLadderProps) {
  const [position, setPosition] = useState<{ top: number; left: number } | null>(null);
  const popupRef = useRef<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    if (!open || !anchor || !popupRef.current) {
      setPosition(null);
      return;
    }
    const rect = anchor.getBoundingClientRect();
    // Default: to the left of the row, vertically centered.
    let left = rect.left - LADDER_WIDTH - 10;
    if (left < VIEWPORT_PAD) {
      // Flip to the right side when the row is near the left edge.
      left = Math.min(
        window.innerWidth - LADDER_WIDTH - VIEWPORT_PAD,
        rect.right + 10,
      );
    }
    // Vertically center on the row, then clamp so neither the top
    // nor the bottom escapes the viewport. We compute the box's top
    // edge directly (no `translateY(-50%)`) so the clamp math is
    // simple: a popup taller than the viewport sticks to the top
    // and gets internal scroll via `max-height` in the CSS.
    const popupHeight = popupRef.current.offsetHeight;
    const rowCenter = rect.top + rect.height / 2;
    let top = rowCenter - popupHeight / 2;
    const maxTop = window.innerHeight - popupHeight - VIEWPORT_PAD;
    if (maxTop < VIEWPORT_PAD) {
      // Popup taller than the viewport — pin to the top.
      top = VIEWPORT_PAD;
    } else {
      top = Math.max(VIEWPORT_PAD, Math.min(top, maxTop));
    }
    setPosition({ top, left });
    // `description` + `whyThisMatters` affect the rendered height,
    // so re-measure when they change; same for `objectOverrides`,
    // `cascadeFallback`, and the layers (length isn't truly stable
    // but for our setting it's enough to depend on the open/anchor
    // pair plus the description-affecting props).
  }, [open, anchor, description, whyThisMatters, cascadeFallback, objectOverrides, objectTier]);

  if (!open) return null;

  const body = (
    <div
      ref={popupRef}
      className="cascade-ladder"
      style={{
        position: "fixed",
        top: position?.top ?? 0,
        left: position?.left ?? 0,
        width: LADDER_WIDTH,
        // Hide until the position measurement settles so the
        // popup doesn't visibly jump from its initial (0,0) frame
        // to its final clamped frame.
        visibility: position ? "visible" : "hidden",
      }}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      role="tooltip"
      aria-label={`Cascade for ${settingLabel}`}
    >
      <div className="ladder-title">
        Cascade · {settingLabel || settingKey}
      </div>
      {(objectTier
        ? [...LAYER_ORDER, { id: "object" as CascadeLayer, label: objectTier.label }]
        : LAYER_ORDER
      ).map(({ id, label }) => {
        const v = layers.get(id) ?? null;
        const defined = v !== null;
        const isWinner = id === winningLayer;
        return (
          <div
            key={id}
            className={`ladder-row${isWinner ? " winner" : ""}${defined ? "" : " empty"}`}
            data-layer={id}
            style={{ ["--row-hue" as string]: String(LAYER_HUE[id]) }}
          >
            <span
              className="l-dot"
              style={{
                width: 8,
                height: 8,
                borderRadius: "50%",
                background: defined
                  ? `hsl(${LAYER_HUE[id]} 70% 55%)`
                  : "transparent",
                border: defined ? "none" : "1px dashed var(--border-strong)",
                flexShrink: 0,
              }}
              aria-hidden
            />
            <span className="l-name" title={label}>{label}</span>
            <span className="l-val" style={defined ? undefined : { fontStyle: "italic", color: "var(--text-dim)" }}>
              {defined ? v : "—"}
            </span>
            {isWinner && <span aria-hidden>✓</span>}
          </div>
        );
      })}

      {cascadeFallback !== null && (
        <>
          <div className="ladder-fallback-sep">cascade fallback</div>
          <div className="ladder-row">
            <span
              className="l-dot"
              style={{
                width: 8,
                height: 8,
                borderRadius: "50%",
                background: `hsl(${LAYER_HUE.cascade} 0% 50%)`,
                opacity: 0.5,
                flexShrink: 0,
              }}
              aria-hidden
            />
            <span className="l-name">reverts to</span>
            <span className="l-val">{cascadeFallback}</span>
          </div>
        </>
      )}

      {objectOverrides.length > 0 && (
        <>
          <div className="ladder-objects-sep">
            {objectOverrides.length} object
            {objectOverrides.length === 1 ? "" : "s"} override
          </div>
          {objectOverrides.map((o) => (
            <div key={o.id} className="ladder-row obj-row">
              <span
                className="l-dot"
                style={{
                  width: 8,
                  height: 8,
                  borderRadius: "50%",
                  background: o.color || "#888",
                  flexShrink: 0,
                }}
                aria-hidden
              />
              <span className="l-name" title={o.name}>
                {o.name}
              </span>
              <span className="l-val">{o.value}</span>
            </div>
          ))}
        </>
      )}

      {(description || whyThisMatters) && (
        <div className="ladder-description">
          {description && (
            <div className="ladder-description-body">{description}</div>
          )}
          {whyThisMatters && (
            <div className="ladder-description-why">
              <span className="ladder-description-why-heading">tip</span>
              {whyThisMatters}
            </div>
          )}
        </div>
      )}
    </div>
  );

  return createPortal(body, document.body);
}

/** Hook owning the hover-open / close-schedule lifecycle. Returns
 *  the wire-up handlers + a `register(el)` callback to attach to
 *  the anchor's React ref. Mockup mirror at SettingsPanel.jsx
 *  :206-219. */
export function useLadderHover() {
  const [open, setOpen] = useState(false);
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const openLadder = (el: HTMLElement) => {
    if (closeTimer.current) {
      clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
    setAnchor(el);
    setOpen(true);
  };

  const scheduleClose = () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
    closeTimer.current = setTimeout(() => setOpen(false), 120);
  };

  useEffect(() => () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
  }, []);

  return { open, anchor, openLadder, scheduleClose };
}
