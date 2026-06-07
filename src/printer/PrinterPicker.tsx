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
import type {
  ConnectionStatus,
  ConnectionSummary,
} from "../driver/useDriverConnections";
import type { PlateId } from "../viewport/types";

/** Tooltip copy for each picker-chip status — mirrors the mockup's
 *  CONN_LABELS map. */
const CONN_LABELS: Record<ConnectionStatus, string> = {
  none: "No connection configured",
  connecting: "Connecting…",
  connected: "Connected",
  failed: "Connection failed",
};

export interface PrinterPickerProps {
  /** Active plate the picker rebinds. `null` disables the
   * picker (e.g. before the snapshot lands). */
  plateId: PlateId | null;
  /** All registered printer instances, in declaration order. */
  instances: PrinterInstance[];
  /** Currently-bound PrinterInstance id, or `null` for unbound. */
  activeInstanceId: string | null;
  /** Per-(instance.id) auto-connection summary. Drives the chip's
   *  bottom-row status dot (none / connecting / connected /
   *  failed) and the per-row dot in the popover. */
  connections: Record<string, ConnectionSummary>;
  /** Opens the add-printer modal. */
  onAddPrinter: () => void;
  /** Opens the per-printer settings modal for the row's instance.
   * Called when the user clicks the cog next to a printer. The
   * App-level handler mounts `PrinterSettingsModal` scoped to
   * `instanceId`. */
  onEditPrinter?: (instanceId: string) => void;
}

export function PrinterPicker({
  plateId,
  instances,
  activeInstanceId,
  connections,
  onAddPrinter,
  onEditPrinter,
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
  const activeSummary =
    activeInstance != null ? connections[activeInstance.id] : null;
  const activeStatus: ConnectionStatus = activeSummary?.status ?? "none";
  // For `failed`, append the reconciler's reason ("host unreachable",
  // "access denied", …) so the user can tell what went wrong without
  // re-opening the settings modal. Other statuses use the static
  // label — no extra context to surface.
  const activeTooltip =
    activeStatus === "failed" && activeSummary?.reason
      ? `${CONN_LABELS.failed}: ${activeSummary.reason}`
      : CONN_LABELS[activeStatus];

  return (
    <div
      className={`config-chip-wrap config-chip-wrap-printer${
        onEditPrinter && activeInstance ? " is-split" : ""
      }`}
      ref={wrapRef}
    >
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
        <span className="chip-value-row">
          <span className="chip-value">{chipLabel}</span>
          {activeInstance != null && (
            <span
              className={`conn-indicator conn-${activeStatus}`}
              title={activeTooltip}
              aria-label={activeTooltip}
            />
          )}
        </span>
      </button>
      {/* Split-button gear: opens the active printer's machine settings
          directly, so per-printer machine config is one click from the chip
          rather than buried in the picker menu. Self-hides when no printer is
          bound (nothing to configure). */}
      {onEditPrinter && activeInstance && (
        <button
          type="button"
          className="chip-gear"
          onClick={(e) => {
            e.stopPropagation();
            onEditPrinter(activeInstance.id);
          }}
          title={`Machine settings — ${chipLabel}\nG-code, limits, connection`}
          aria-label={`Machine settings for ${chipLabel}`}
        >
          <svg width="15" height="15" viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <circle cx="8" cy="8" r="2.3" stroke="currentColor" strokeWidth="1.3" />
            <path
              d="M8 1.4v2M8 12.6v2M1.4 8h2M12.6 8h2M3.3 3.3l1.4 1.4M11.3 11.3l1.4 1.4M3.3 12.7l1.4-1.4M11.3 4.7l1.4-1.4"
              stroke="currentColor"
              strokeWidth="1.3"
              strokeLinecap="round"
            />
          </svg>
        </button>
      )}
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
              <div
                key={inst.id}
                role="menuitemradio"
                aria-checked={isActive}
                className={`ptpm-item ptpm-row${isActive ? " active" : ""}`}
              >
                <button
                  type="button"
                  className="ptpm-row-main"
                  onClick={() => selectInstance(inst.id)}
                  title={`Bind ${inst.display_name} to this plate`}
                >
                  <span className="ptpm-name">{inst.display_name}</span>
                  <span className="ptpm-detail">{model}</span>
                </button>
                {onEditPrinter && (
                  <button
                    type="button"
                    className="ptpm-cog"
                    onClick={(e) => {
                      e.stopPropagation();
                      setOpen(false);
                      onEditPrinter(inst.id);
                    }}
                    title={`Settings for ${inst.display_name}`}
                    aria-label={`Settings for ${inst.display_name}`}
                  >
                    <svg width="13" height="13" viewBox="0 0 14 14" fill="none">
                      <path
                        d="M7 4.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM7 1v1.5M7 11.5V13M3.5 3.5l1 1M9.5 9.5l1 1M1 7h1.5M11.5 7H13M3.5 10.5l1-1M9.5 4.5l1-1"
                        stroke="currentColor"
                        strokeWidth="1.2"
                        strokeLinecap="round"
                      />
                    </svg>
                  </button>
                )}
              </div>
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
