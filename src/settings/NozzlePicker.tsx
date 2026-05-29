// Installed-nozzle picker.
//
// Chip + popover, one per extruder. Chip label is `Nozzle` for
// single-extruder printers and `Nozzle T1` / `Nozzle T2` / … for
// multi-extruder printers (so multiple chips in the same row read
// as a numbered set; 1-based for display, 0-based internally).
// Value shown on the chip is the diameter in millimetres (e.g.
// `0.4 mm`). The popover lists every diameter the printer profile
// bundled nozzle fragments for.
//
// Material is out of scope for the MVP — the picker only writes
// diameter swaps; the backend mutation preserves whatever material
// was set.

import { useEffect, useRef, useState } from "react";

export interface NozzlePickerProps {
  /** 0-based extruder position. Multi-extruder printers show
   *  this in the chip label as `Nozzle T<idx + 1>` (1-based for
   *  display). */
  extruderIdx: number;
  /** Total extruder count — drives whether the label is `Nozzle` (1) or
   *  `Nozzle T<idx + 1>` (>1) in the default form. */
  totalExtruders: number;
  /** Drop the `Nozzle` prefix from the chip label and popover title;
   *  the chip reads just `T<idx + 1>`. Used when chips sit under the
   *  "Nozzles" section divider in the expanded layout (3+ extruders),
   *  where the section header already carries the noun. */
  compact?: boolean;
  /** Currently-installed diameter on this extruder. String symbol
   *  ("0.4"), not a number — see [NozzleSku.diameter] for why. */
  value: string;
  /** Diameters the printer profile bundled nozzle fragments for. */
  diameters: readonly string[];
  onChange: (next: string) => void;
  /** Diameter the printer treats as its default (typically the
   *  `Toolhead.default_nozzle_diameter`). Renders the `default` badge
   *  on the matching popover entry. */
  printerDefault?: string | null;
  disabled?: boolean;
}

/** Format a diameter for display: "0.4" → "0.4 mm". The
 *  diameter is already a clean symbol from the source list
 *  (0.2 / 0.4 / 0.6 / 0.8 etc.); the formatter just appends
 *  the unit. */
function formatDiameter(d: string): string {
  return `${d} mm`;
}

export function NozzlePicker({
  extruderIdx,
  totalExtruders,
  compact = false,
  value,
  diameters,
  onChange,
  printerDefault = null,
  disabled = false,
}: NozzlePickerProps): React.ReactElement {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDocClick = (e: MouseEvent) => {
      if (!wrapRef.current) return;
      if (!wrapRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, [open]);

  const pick = (next: string): void => {
    setOpen(false);
    if (next !== value) onChange(next);
  };

  // 1-based for display; the extruderIdx prop stays 0-based to
  // match every other API surface that addresses extruders by
  // position.
  const chipLabel = compact
    ? `T${extruderIdx + 1}`
    : totalExtruders > 1
      ? `Nozzle T${extruderIdx + 1}`
      : "Nozzle";
  const isDefault = printerDefault != null && value === printerDefault;

  return (
    <div className="config-chip-wrap" ref={wrapRef}>
      <button
        type="button"
        className="config-chip config-chip-nozzle"
        onClick={() => setOpen((v) => !v)}
        disabled={disabled || diameters.length === 0}
        title={
          diameters.length === 0
            ? "Printer has no bundled nozzle fragments"
            : `${chipLabel} — click to change`
        }
        aria-haspopup="menu"
        aria-expanded={open}
        data-default={isDefault}
      >
        <span className="config-chip-top">
          <span className="chip-label">{chipLabel}</span>
          <span className="chev" aria-hidden>
            ▾
          </span>
        </span>
        <span className="chip-value">{formatDiameter(value)}</span>
      </button>
      {open && (
        <div className="printer-picker-menu" role="menu">
          <div className="ptpm-title">{chipLabel}</div>
          {diameters.map((d) => {
            const isActive = d === value;
            return (
              <button
                key={d}
                type="button"
                role="menuitemradio"
                aria-checked={isActive}
                className={`ptpm-item${isActive ? " active" : ""}`}
                onClick={() => pick(d)}
              >
                <span className="ptpm-name">{formatDiameter(d)}</span>
                {d === printerDefault && (
                  <span className="ptpm-detail">default</span>
                )}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
