// U1 toolhead strip — per-toolhead card with chip + material + temp.
//
// U1 is a 4-toolhead CoreXY toolchanger. Each toolhead docks
// independently and has its own permanent filament + own hotend
// temp; the "mounted" toolhead is the one currently in the carriage
// driving the print. The strip mirrors [`BambuAmsStrip`]'s visual
// conventions (color chip, active ring, dashed empty) but renders
// as 4 vertical mini-cards rather than a chip row — each card has
// to fit a material-type label + temp readout per toolhead.
//
// Per-toolhead temp readouts live here (not in the panel's shared
// `TempsLine`) because the U1 reports 4 independent extruders;
// `TempsLine` was authored against the single-nozzle Bambu shape
// and is reduced to bed-only for U1 contexts.

import { cssColorFromHex } from "./colorUtils";
import type { TempReading, Temps, U1Extra, U1Filament } from "./types";

/** U1 ships 4 toolheads. Render fixed-size — empty cells stand in
 * for any slot the printer hasn't reported a filament for. */
const TOOLHEAD_COUNT = 4;

/** Truncate material-type strings (`"PLA"`, `"Carbon Fiber PLA"`)
 * to a chip-sized label. 6 chars fits the 56px card without
 * wrapping for the common materials; longer custom types get
 * truncated. */
const MATERIAL_LABEL_LIMIT = 6;

export interface U1ToolheadStripProps {
  extra: U1Extra;
  temps: Temps;
}

export interface CellView {
  index: number;
  cssColor: string | null;
  /** Truncated material label, or `null` when no filament reported. */
  materialLabel: string | null;
  /** `"CUR/SET°"` or `"—"` when no temp reading. */
  tempReadout: string;
  isMounted: boolean;
  /** Full hover/screen-reader label — material type un-truncated,
   * temp inline, mounted state called out. */
  ariaLabel: string;
}

/** Pure projection: `U1Extra` + `Temps` → cell descriptors. Extracted
 * for testability, mirrors `BambuAmsStrip::chipsFromAms`. */
export function cellsFromU1(extra: U1Extra, temps: Temps): CellView[] {
  const out: CellView[] = [];
  for (let i = 0; i < TOOLHEAD_COUNT; i++) {
    out.push(
      cellFromIndex(
        i,
        extra.toolhead_filaments[i] ?? null,
        temps.nozzles[i] ?? null,
        extra.mounted_toolhead,
      ),
    );
  }
  return out;
}

function cellFromIndex(
  i: number,
  filament: U1Filament | null,
  nozzle: TempReading | null,
  mounted: number | null,
): CellView {
  const isMounted = mounted != null && mounted === i;
  const tempReadout = formatTemp(nozzle);
  const mountedSuffix = isMounted ? " (mounted)" : "";
  // Display label is 1-based — the cell index `i` and the firmware's
  // `mounted_toolhead` stay 0-based internally.
  const displayLabel = `T${i + 1}`;
  if (filament == null) {
    return {
      index: i,
      cssColor: null,
      materialLabel: null,
      tempReadout,
      isMounted,
      ariaLabel: `${displayLabel}${mountedSuffix}: empty · ${tempReadout}`,
    };
  }
  return {
    index: i,
    cssColor: cssColorFromHex(filament.color),
    materialLabel: filament.material_type.slice(0, MATERIAL_LABEL_LIMIT),
    tempReadout,
    isMounted,
    ariaLabel: `${displayLabel}${mountedSuffix}: ${filament.material_type} · ${tempReadout}`,
  };
}

function formatTemp(reading: TempReading | null): string {
  if (reading == null) return "—";
  return `${Math.round(reading.current)}/${Math.round(reading.target)}°`;
}

export function U1ToolheadStrip({
  extra,
  temps,
}: U1ToolheadStripProps): React.JSX.Element {
  const cells = cellsFromU1(extra, temps);
  return (
    <div className="flex gap-1.5 items-stretch" aria-label="U1 toolheads">
      {cells.map((c) => (
        <div
          key={c.index}
          className={`w-20 flex flex-col items-center gap-0.5 px-1 py-1 rounded-sm border border-border text-text-muted ${
            c.cssColor == null ? "border-dashed" : ""
          } ${c.isMounted ? "ring-2 ring-accent ring-offset-1" : ""}`}
          title={c.ariaLabel}
          aria-label={c.ariaLabel}
        >
          <span
            className={`w-5 h-5 rounded-sm border border-border ${
              c.cssColor == null ? "border-dashed bg-transparent" : ""
            }`}
            style={c.cssColor ? { background: c.cssColor } : undefined}
          />
          <span className="text-[10px] font-mono leading-tight whitespace-nowrap">
            T{c.index + 1} {c.materialLabel ?? "—"} {c.tempReadout}
          </span>
        </div>
      ))}
    </div>
  );
}
