// Printer picker chip + popover menu.
//
// Lists the user's registered `PrinterInstance`s (by display name)
// next to the slot tabs in the SettingsPanel config strip. Picking
// one rebinds the active plate via `scene_rebind_plate_printer` —
// the rest of the panel re-resolves the cascade naturally because
// `useCascadeResolve`'s context-key flips when the bound instance
// changes.
//
// A "+ New printer…" row at the bottom of the menu opens the
// add-printer modal, which the App-level handler wires up.

import { useEffect, useRef, useState } from "react";
import { usePrinterCatalog } from "./usePrinterCatalog";
import { rebindPlatePrinter } from "./printerCommands";
import type { PrinterInstance } from "./printerInstance";
import type { PlateId } from "../viewport/types";

export interface PrinterPickerProps {
  /** Active plate the picker rebinds. `null` disables the
   * picker (e.g. before the snapshot lands). */
  plateId: PlateId | null;
  /** All registered printer instances, in declaration order. */
  instances: PrinterInstance[];
  /** Currently-bound PrinterInstance id, or `null` for unbound. */
  activeInstanceId: string | null;
  /** Opens the add-printer modal. */
  onAddPrinter: () => void;
}

export function PrinterPicker({
  plateId,
  instances,
  activeInstanceId,
  onAddPrinter,
}: PrinterPickerProps) {
  const { entries: catalog } = usePrinterCatalog();
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

  const selectInstance = (instanceId: string): void => {
    if (plateId === null) return;
    setOpen(false);
    void rebindPlatePrinter(plateId, instanceId).catch((err) => {
      console.error("[printer] rebindPlatePrinter failed", err);
    });
  };

  const handleAddNew = (): void => {
    setOpen(false);
    onAddPrinter();
  };

  const activeInstance = instances.find((i) => i.id === activeInstanceId) ?? null;
  const chipLabel = activeInstance?.display_name ?? "No printer";

  return (
    <div className="config-chip-wrap" ref={wrapRef}>
      <button
        type="button"
        className="config-chip config-chip-printer"
        onClick={() => setOpen((v) => !v)}
        disabled={plateId === null}
        title={
          plateId === null
            ? "Select a plate to change its printer"
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
          {instances.length === 0 && (
            <div className="ptpm-empty">No printers yet.</div>
          )}
          {instances.map((inst) => {
            const isActive = inst.id === activeInstanceId;
            const model =
              catalog.find((e) => e.identity === inst.vendor_profile_ref)
                ?.profile.model ?? inst.vendor_profile_ref;
            return (
              <button
                key={inst.id}
                type="button"
                role="menuitemradio"
                aria-checked={isActive}
                className={`ptpm-item${isActive ? " active" : ""}`}
                onClick={() => selectInstance(inst.id)}
              >
                <span className="ptpm-name">{inst.display_name}</span>
                <span className="ptpm-detail">{model}</span>
              </button>
            );
          })}
          <button
            type="button"
            className="ptpm-item ptpm-add"
            onClick={handleAddNew}
          >
            <span className="ptpm-name">＋ New printer…</span>
          </button>
        </div>
      )}
    </div>
  );
}
