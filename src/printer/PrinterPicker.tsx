// Printer picker chip + popover menu (PR-5-4).
//
// Surfaces the bundled printer catalog as a click-to-open menu
// next to the slot tabs in the SettingsPanel config strip.
// Selecting a printer rebinds the active plate via
// `scene_rebind_plate_printer` — the rest of the panel re-resolves
// the cascade naturally because `useCascadeResolve`'s context-key
// flips when the printer profile changes.
//
// Mirrors the design's `.printer-picker-menu` pattern: small
// popover with one `.ptpm-item` row per catalog entry, active row
// highlighted. Per-printer build-plate picking comes via the
// existing build-plate selector (PR-4-5); the picker selects the
// first supported plate when the user picks a different printer
// (the rebind mutation requires a valid build-plate identity).

import { useEffect, useRef, useState } from "react";
import { usePrinterCatalog } from "./usePrinterCatalog";
import { rebindPlatePrinter } from "./printerCommands";
import type { PlateId, PrinterBinding } from "../viewport/types";

export interface PrinterPickerProps {
  /** Active plate the picker rebinds. `null` disables the
   * picker (e.g. before the snapshot lands). */
  plateId: PlateId | null;
  /** Currently-bound printer + build plate. `null` when the plate
   * is unbound — the chip surfaces "no printer". */
  binding: PrinterBinding | null;
}

export function PrinterPicker({ plateId, binding }: PrinterPickerProps) {
  const { entries, loading, error } = usePrinterCatalog();
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  // Click-outside-to-close. Listen at document level so a click on
  // a non-picker chip / row also dismisses.
  useEffect(() => {
    if (!open) return;
    const onDocClick = (e: MouseEvent) => {
      if (!wrapRef.current) return;
      if (!wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, [open]);

  const selectPrinter = (identity: string): void => {
    if (plateId === null) return;
    const entry = entries.find((e) => e.identity === identity);
    if (!entry) return;
    // Keep the existing build plate if the new printer supports
    // it; otherwise fall back to the printer's first supported
    // plate (the rebind mutation requires a valid identity).
    const previousPlate = binding?.build_plate_identity;
    const buildPlate =
      previousPlate &&
      entry.profile.supported_build_plates.includes(previousPlate)
        ? previousPlate
        : entry.profile.supported_build_plates[0];
    if (!buildPlate) {
      console.error(
        `[printer] catalog entry "${identity}" has no supported_build_plates`,
      );
      setOpen(false);
      return;
    }
    setOpen(false);
    void rebindPlatePrinter(plateId, identity, buildPlate).catch((err) => {
      console.error("[printer] rebindPlatePrinter failed", err);
    });
  };

  const activeEntry =
    binding == null
      ? null
      : entries.find((e) => e.identity === binding.printer_identity) ?? null;
  const chipLabel =
    activeEntry?.profile.model ?? binding?.printer_identity ?? "No printer";

  return (
    <div className="config-chip-wrap" ref={wrapRef}>
      <button
        type="button"
        className="config-chip config-chip-printer"
        onClick={() => setOpen((v) => !v)}
        disabled={plateId === null || loading}
        title={
          plateId === null
            ? "Select a plate to change its printer"
            : loading
              ? "Loading printer catalog…"
              : "Change printer for this plate"
        }
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <span className="config-chip-top">
          <span className="chip-label">Printer</span>
          <span className="chev" aria-hidden>
            ▾
          </span>
        </span>
        <span className="chip-value">{chipLabel}</span>
      </button>
      {open && (
        <div className="printer-picker-menu" role="menu">
          <div className="ptpm-title">Printer</div>
          {error && (
            <div className="ptpm-error" role="alert">
              {error}
            </div>
          )}
          {entries.map((entry) => {
            const isActive = entry.identity === binding?.printer_identity;
            return (
              <button
                key={entry.identity}
                type="button"
                role="menuitemradio"
                aria-checked={isActive}
                className={`ptpm-item${isActive ? " active" : ""}`}
                onClick={() => selectPrinter(entry.identity)}
              >
                <span className="ptpm-name">{entry.profile.model}</span>
                <span className="ptpm-detail">
                  {entry.profile.slot_count} slot
                  {entry.profile.slot_count === 1 ? "" : "s"}
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
