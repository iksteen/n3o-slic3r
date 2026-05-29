// Quality picker — selects the process fragment ("0.20mm Standard",
// "0.16mm Optimal", …) the cascade composer hands to the slicer.
//
// Chip + popover, mirroring NozzlePicker's shape. Chip's primary
// line is the process display name; secondary line is the
// fragment's `layer_height` in millimetres. Popover lists every
// process the backend reports available for the active
// (printer, nozzle); selection writes back via
// `setInstanceQualityProfile`.

import { useEffect, useRef, useState } from "react";
import type { ProcessFragmentSummary } from "./processFragment";

export interface QualityPickerProps {
  /** Currently-selected process slug — matched against
   *  `options[].slug`. If no match, the chip falls back to
   *  rendering the raw slug. */
  value: string;
  /** Backend-supplied list of available processes for the active
   *  (printer, nozzle). Empty → chip stays disabled. */
  options: readonly ProcessFragmentSummary[];
  onChange: (next: string) => void;
  disabled?: boolean;
}

/** Render a fragment's layer height as `"0.20 mm"`. */
function formatLayerHeight(mm: number | null): string {
  if (mm == null) return "";
  // Two-decimal canonical form so 0.2 → "0.20 mm" matches the
  // mockup's chip styling.
  return `${mm.toFixed(2)} mm`;
}

/** Strip the leading layer-height token from an upstream process
 *  name so the chip + menu can show name + height as two distinct
 *  pieces (matching the mockup) without printing the height twice.
 *  Upstream conventions vary — BBL writes "0.20mm Standard",
 *  Snapmaker writes "0.20 Standard" — so accept both. Returns the
 *  original string when no recognizable prefix is present. */
function stripLayerHeightPrefix(name: string): string {
  return name.replace(/^\d+(?:\.\d+)?\s*(?:mm)?\s+/i, "");
}

export function QualityPicker({
  value,
  options,
  onChange,
  disabled = false,
}: QualityPickerProps): React.ReactElement {
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

  const current = options.find((o) => o.slug === value);
  const empty = options.length === 0;
  // If the active slug isn't in the available set, the picker shows
  // the slug as a fallback label so the user can still see what's
  // bound (and pick a different process to fix the mismatch).
  const chipName = current
    ? stripLayerHeightPrefix(current.display_name)
    : value;
  const chipSub = current ? formatLayerHeight(current.layer_height_mm) : "";

  return (
    <div className="sp-quality-chip-wrap" ref={wrapRef}>
      <button
        type="button"
        className={`sp-quality-chip${open ? " open" : ""}`}
        onClick={() => setOpen((v) => !v)}
        disabled={disabled || empty}
        title={
          empty
            ? "No process fragments available for this (printer, nozzle)"
            : `Quality — click to change`
        }
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        <span className="sp-quality-chip-main">
          <span className="sp-quality-chip-name">{chipName}</span>
          {chipSub && <span className="sp-quality-chip-h">{chipSub}</span>}
        </span>
        <span className="chev" aria-hidden>
          ▾
        </span>
      </button>
      {open && (
        <div className="sp-quality-menu" role="listbox">
          {options.map((o) => {
            const isActive = o.slug === value;
            return (
              <button
                key={o.slug}
                type="button"
                role="option"
                aria-selected={isActive}
                className={`sp-quality-item${isActive ? " active" : ""}`}
                onClick={() => pick(o.slug)}
              >
                <span className="sp-quality-item-name">
                  {stripLayerHeightPrefix(o.display_name)}
                </span>
                {o.layer_height_mm != null && (
                  <span className="sp-quality-item-h">
                    {formatLayerHeight(o.layer_height_mm)}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
