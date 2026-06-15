// One row in the materials sub-section — represents a logical model
// material (M1, M2…) and lets the user route it to a physical slot on
// the bound printer. Ported from `docs/dev/design/SettingsPanel.jsx`'s
// MaterialChip.
//
// The chip carries the full chain at a glance:
//   id (M1) → arrow → slot short label → swatch → filament name → ×use-count
// Click opens a popover listing every slot on the printer with its
// current filament; clicking a slot rewrites the plate's
// `material_to_slot` map.

import { useRef, useState } from "react";
import { usePopoverDismiss } from "../ui/usePopoverDismiss";
import {
  deriveSlotShortLabel,
  type FlatSlotOption,
  type SlotBinding,
  type SlotRef,
} from "../printer/printerInstance";
import type { FilamentSummary } from "./filamentSummary";

const UNASSIGNED_SWATCH = "#9ca3af";

const ChevronChip = (): React.JSX.Element => (
  <svg
    width="9"
    height="9"
    viewBox="0 0 10 10"
    fill="none"
    style={{ opacity: 0.55, flexShrink: 0 }}
    aria-hidden
  >
    <path
      d="M2 4l3 3 3-3"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

export interface MaterialChipProps {
  /** 1-based material index (M1, M2, …). */
  material: number;
  /** Slot currently routed for this material, or null when unmapped. */
  current: FlatSlotOption | null;
  /** Every slot on the bound printer (target choices for the popover). */
  slots: readonly FlatSlotOption[];
  /** Total extruders on the instance — needed for `deriveSlotShortLabel`. */
  totalExtruders: number;
  /** Per-extruder slot vectors — needed for `deriveSlotShortLabel`. */
  extruderSlots: readonly (readonly SlotBinding[])[];
  /** Resolved filament details for each slot, keyed by identity. */
  filamentByIdentity: Map<string, FilamentSummary>;
  /** Object count on the plate that references this material. */
  useCount: number;
  onPickSlot: (slot: SlotRef) => void;
  onClear: () => void;
}

function shortLabelFor(
  slot: FlatSlotOption,
  totalExtruders: number,
  extruderSlots: readonly (readonly SlotBinding[])[],
): string {
  const ext = extruderSlots[slot.ref.extruder];
  if (!ext) return slot.label;
  return deriveSlotShortLabel(
    slot.ref.extruder,
    totalExtruders,
    slot.ref.slot,
    ext,
  );
}

export function MaterialChip({
  material,
  current,
  slots,
  totalExtruders,
  extruderSlots,
  filamentByIdentity,
  useCount,
  onPickSlot,
  onClear,
}: MaterialChipProps): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  usePopoverDismiss(wrapRef, () => setOpen(false), open);

  const currentFil = current?.filament_identity
    ? (filamentByIdentity.get(current.filament_identity) ?? null)
    : null;
  const currentFilLabel = currentFil
    ? currentFil.display_name
    : current?.filament_identity
      ? current.filament_identity
      : "unmapped";
  const currentShort = current
    ? shortLabelFor(current, totalExtruders, extruderSlots)
    : "—";
  const swatch = current?.color ?? UNASSIGNED_SWATCH;

  const id = `M${material}`;
  const tooltip = current
    ? `${id} → ${current.label} (${currentFilLabel}) · ${useCount} object${useCount !== 1 ? "s" : ""}`
    : `${id} → unmapped · ${useCount} object${useCount !== 1 ? "s" : ""}`;

  return (
    <div className="config-chip-wrap" ref={wrapRef}>
      <button
        type="button"
        className="material-chip"
        onClick={() => setOpen((v) => !v)}
        title={tooltip}
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <span className="material-id">{id}</span>
        <span className="material-arrow" aria-hidden>
          →
        </span>
        <span className="material-slot">{currentShort}</span>
        <span
          className="fil-swatch"
          style={
            current?.filament_identity
              ? { background: swatch, border: "none" }
              : { background: "transparent", border: "1px dashed currentColor" }
          }
        />
        <span className="fil-label">{currentFilLabel}</span>
        {/* A painted material has no object directly assigned to it
            (it's applied per-face), so `×0` would be noise — omit it. */}
        {useCount > 0 && <span className="fil-count">×{useCount}</span>}
        <ChevronChip />
      </button>
      {open && (
        <div
          className="printer-picker-menu material-menu"
          role="menu"
          onClick={(e) => e.stopPropagation()}
        >
          <div className="ptpm-title">Route {id} to slot…</div>
          {slots.map((s) => {
            const fil = s.filament_identity
              ? (filamentByIdentity.get(s.filament_identity) ?? null)
              : null;
            const filLabel = fil
              ? fil.display_name
              : s.filament_identity
                ? s.filament_identity
                : "empty";
            const isActive =
              !!current &&
              current.ref.extruder === s.ref.extruder &&
              current.ref.slot === s.ref.slot;
            return (
              <button
                key={`${s.ref.extruder}-${s.ref.slot}`}
                type="button"
                className={`ptpm-item ptpm-row${isActive ? " active" : ""}`}
                onClick={() => {
                  onPickSlot(s.ref);
                  setOpen(false);
                }}
              >
                <span className="ptpm-name">
                  <span
                    className="ptpm-swatch"
                    style={
                      s.filament_identity
                        ? { background: s.color ?? UNASSIGNED_SWATCH, border: "none" }
                        : {
                            background: "transparent",
                            border: "1px dashed currentColor",
                          }
                    }
                  />
                  {s.label}
                </span>
                <span className="ptpm-detail">{filLabel}</span>
              </button>
            );
          })}
          {current && (
            <button
              type="button"
              className="ptpm-item ptpm-row ptpm-clear"
              onClick={() => {
                onClear();
                setOpen(false);
              }}
            >
              <span className="ptpm-name">— unmap —</span>
            </button>
          )}
        </div>
      )}
    </div>
  );
}
