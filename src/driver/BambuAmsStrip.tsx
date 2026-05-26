// AMS slot strip — colored chips per loaded spool (PR-7a-7).
//
// One row per AMS unit (the A1 mini ships with one 4-tray AMS lite;
// X1C / P1S can have up to four). Each tray renders as a small
// colored chip; empty trays render dashed-outline. The active slot
// gets a ring so the user knows which one is feeding.
//
// Color values arrive as `RRGGBBAA` hex without `#` (Bambu's wire
// shape); we prepend `#` for CSS consumption. Sentinel "empty
// spool" identity (per PR-7a-4) shows up as `identity == null`.

import { cssColorFromHex } from "./colorUtils";
import type { AmsState, AmsTray, AmsUnit } from "./types";

export interface BambuAmsStripProps {
  ams: AmsState | null;
}

interface ChipView {
  trayId: number;
  unitId: number;
  cssColor: string | null;
  trayType: string | null;
  cssLabel: string | null;
  isActive: boolean;
}

/** Pure projection from the AMS wire shape to render-ready chip
 * descriptors. Extracted so the test suite can exercise the color
 * normalization + active-slot detection without rendering DOM. */
export function chipsFromAms(ams: AmsState): ChipView[] {
  const out: ChipView[] = [];
  for (const unit of ams.units) {
    for (const tray of unit.trays) {
      out.push(chipFromTray(unit, tray, ams.active_slot));
    }
  }
  return out;
}

function chipFromTray(
  unit: AmsUnit,
  tray: AmsTray,
  activeSlot: number | null,
): ChipView {
  const isActive = activeSlot != null && activeSlot === tray.id;
  if (!tray.identity) {
    return {
      trayId: tray.id,
      unitId: unit.id,
      cssColor: null,
      trayType: null,
      cssLabel: null,
      isActive,
    };
  }
  return {
    trayId: tray.id,
    unitId: unit.id,
    cssColor: cssColorFromHex(tray.identity.color),
    trayType: tray.identity.tray_type,
    cssLabel: `${tray.identity.tray_type} · #${tray.identity.color}`,
    isActive,
  };
}

export function BambuAmsStrip({ ams }: BambuAmsStripProps): React.JSX.Element | null {
  if (ams == null) return null;
  const chips = chipsFromAms(ams);
  if (chips.length === 0) return null;
  return (
    <div className="flex gap-1.5 items-center" aria-label="AMS slots">
      {chips.map((c) => (
        <span
          key={`${c.unitId}-${c.trayId}`}
          className={`w-5 h-5 rounded-sm border border-border ${
            c.cssColor == null ? "border-dashed bg-transparent" : ""
          } ${c.isActive ? "ring-2 ring-accent ring-offset-1" : ""}`}
          style={c.cssColor ? { background: c.cssColor } : undefined}
          title={c.cssLabel ?? `Slot ${c.trayId + 1}: empty`}
          aria-label={
            c.cssLabel != null
              ? `Slot ${c.trayId + 1}${c.isActive ? " (active)" : ""}: ${c.cssLabel}`
              : `Slot ${c.trayId + 1}: empty`
          }
        />
      ))}
    </div>
  );
}
