// Build plate selector — chip + popover, matches PrinterPicker
// shape so the two read as a pair in the settings config strip.

import { useEffect, useRef, useState } from "react";

export interface BuildPlateSelectorProps {
  /** All plate identities this printer supports. */
  plates: readonly string[];
  /** Currently selected plate identity. */
  value: string;
  onChange: (next: string) => void;
  /** Plate identity the printer profile considers its default —
   *  renders the `default` badge when the user's selection matches. */
  printerDefault?: string | null;
  disabled?: boolean;
}

export function BuildPlateSelector({
  plates,
  value,
  onChange,
  printerDefault = null,
  disabled = false,
}: BuildPlateSelectorProps) {
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

  const isDefault = printerDefault != null && value === printerDefault;

  return (
    <div className="config-chip-wrap" ref={wrapRef}>
      <button
        type="button"
        className="config-chip config-chip-plate"
        onClick={() => setOpen((v) => !v)}
        disabled={disabled || plates.length === 0}
        title={
          plates.length === 0
            ? "Printer has no build plates"
            : "Change build plate for this plate"
        }
        aria-haspopup="menu"
        aria-expanded={open}
        data-default={isDefault}
      >
        <span className="config-chip-top">
          <span className="chip-label">Plate</span>
          <span className="chev" aria-hidden>
            ▾
          </span>
        </span>
        <span className="chip-value">{value || "—"}</span>
      </button>
      {open && (
        <div className="printer-picker-menu" role="menu">
          <div className="ptpm-title">Build plate</div>
          {plates.map((p) => {
            const isActive = p === value;
            return (
              <button
                key={p}
                type="button"
                role="menuitemradio"
                aria-checked={isActive}
                className={`ptpm-item${isActive ? " active" : ""}`}
                onClick={() => pick(p)}
              >
                <span className="ptpm-name">{p}</span>
                {p === printerDefault && (
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
