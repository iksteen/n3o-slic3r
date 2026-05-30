// Horizontal plate tabs (PR-5-3).
//
// Strip layout: scrollable tab list + trailing "+" button. Each
// tab carries plate icon, name (dblclick-rename), printer label,
// object count, and a close button (when there is more than one
// plate — the backend rejects removing the last plate).
//
// Visual styling lives in `src/index.css` under the `.plate-tabs`,
// `.plate-tab`, `.plate-tab-*` selectors — ported from
// `docs/design/PlateTabs.jsx`. This file is presentation-free; it
// just stamps the right class hooks + wires events.
//
// The strip is a view on top of `usePlateTabs`; mutation goes
// straight to `plateCommands` (no local optimistic state). Backend
// emits the event, hook re-fetches, strip re-renders.

import { useEffect, useRef, useState } from "react";
import { usePlateTabs, type PlateTabView } from "./usePlateTabs";
import {
  addPlate,
  removePlate,
  renamePlate,
  setActivePlate,
} from "./plateCommands";
import type { PlateId } from "../viewport/types";

export interface PlateTabsProps {
  /** True when the workspace is in fleet-monitor (Devices) mode — the
   *  right-aligned Devices tab is the active one and no plate tab is. */
  devicesActive: boolean;
  /** Printer count shown on the Devices tab. */
  deviceCount: number;
  /** Enter Devices mode (right-aligned tab click). */
  onSelectDevices: () => void;
  /** Leave Devices mode when a plate is selected (back to Prepare). */
  onSelectPlate: () => void;
}

export function PlateTabs({
  devicesActive,
  deviceCount,
  onSelectDevices,
  onSelectPlate,
}: PlateTabsProps) {
  const { plates, activePlateId, loading } = usePlateTabs();
  const [editingId, setEditingId] = useState<PlateId | null>(null);
  const [editValue, setEditValue] = useState("");
  const editInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editingId !== null && editInputRef.current) {
      editInputRef.current.focus();
      editInputRef.current.select();
    }
  }, [editingId]);

  const commitRename = (plateId: PlateId): void => {
    const trimmed = editValue.trim();
    if (trimmed.length > 0) {
      void renamePlate(plateId, trimmed).catch((err) => {
        console.error("[plates] renamePlate failed", err);
      });
    }
    setEditingId(null);
  };

  const cancelRename = (): void => {
    setEditingId(null);
  };

  const beginRename = (plate: PlateTabView): void => {
    setEditingId(plate.id);
    setEditValue(plate.name);
  };

  if (loading) {
    return <div className="plate-tabs" aria-hidden />;
  }

  const handleAddPlate = (): void => {
    onSelectPlate();
    void addPlate(null)
      .then((newId) => {
        // Auto-switch to the freshly-added plate so the user
        // lands on the workspace they just opened. Mirrors the
        // intuitive "tab opened in foreground" pattern.
        void setActivePlate(newId).catch((err) =>
          console.error("[plates] setActivePlate after addPlate failed", err),
        );
      })
      .catch((err) => {
        console.error("[plates] addPlate failed", err);
      });
  };

  return (
    <div className="plate-tabs" role="tablist" aria-label="Plates">
      <button
        type="button"
        className="plate-tab-add"
        onClick={handleAddPlate}
        title="New plate"
        aria-label="Add new plate"
      >
        <svg width="13" height="13" viewBox="0 0 14 14" fill="none" aria-hidden>
          <path
            d="M7 2v10M2 7h10"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </svg>
      </button>
      <div className="plate-tabs-scroll">
        {plates.map((plate) => {
          const isActive = !devicesActive && plate.id === activePlateId;
          const isEditing = editingId === plate.id;
          return (
            <div
              key={plate.id}
              role="tab"
              aria-selected={isActive}
              className={`plate-tab${isActive ? " active" : ""}`}
              onClick={() => {
                if (isEditing) return;
                // Selecting a plate leaves Devices mode. Skip the
                // setActivePlate IPC only when it's already active AND
                // we weren't in Devices mode.
                onSelectPlate();
                if (plate.id !== activePlateId) {
                  void setActivePlate(plate.id);
                }
              }}
            >
              <span className="plate-tab-icon" title="Build plate" aria-hidden>
                <svg width="13" height="13" viewBox="0 0 14 14" fill="none">
                  <path
                    d="M1 9l6 3 6-3M1 6l6 3 6-3M1 3l6 3 6-3-6-3-6 3z"
                    stroke="currentColor"
                    strokeWidth="1.1"
                    strokeLinejoin="round"
                  />
                </svg>
              </span>
              {isEditing ? (
                <input
                  ref={editInputRef}
                  className="plate-tab-rename-input"
                  value={editValue}
                  onChange={(e) => setEditValue(e.target.value)}
                  onBlur={() => commitRename(plate.id)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") commitRename(plate.id);
                    if (e.key === "Escape") cancelRename();
                  }}
                  onClick={(e) => e.stopPropagation()}
                />
              ) : (
                <span
                  className="plate-tab-name"
                  onDoubleClick={(e) => {
                    e.stopPropagation();
                    beginRename(plate);
                  }}
                  title="Double-click to rename"
                >
                  {plate.name}
                </span>
              )}
              <span className="plate-tab-divider" aria-hidden />
              <span
                className="plate-tab-printer-display"
                title={
                  plate.printerLabel
                    ? `Assigned to ${plate.printerLabel} — change in the settings panel`
                    : "No printer assigned"
                }
              >
                {plate.printerLabel ?? "—"}
              </span>
              <span className="plate-tab-meta">{plate.objectCount} obj</span>
              {plates.length > 1 && (
                <button
                  type="button"
                  className="plate-tab-close"
                  onClick={(e) => {
                    e.stopPropagation();
                    void removePlate(plate.id).catch((err) => {
                      console.error("[plates] removePlate failed", err);
                    });
                  }}
                  title="Close plate"
                  aria-label={`Close plate ${plate.name}`}
                >
                  <svg width="9" height="9" viewBox="0 0 12 12" fill="none">
                    <path
                      d="M2.5 2.5l7 7M9.5 2.5l-7 7"
                      stroke="currentColor"
                      strokeWidth="1.4"
                      strokeLinecap="round"
                    />
                  </svg>
                </button>
              )}
            </div>
          );
        })}
      </div>
      {/* Right-aligned context tab: switches the workspace into the
          fleet-monitor (Devices) mode. */}
      <button
        type="button"
        role="tab"
        aria-selected={devicesActive}
        className={`plate-tab plate-tab-devices${devicesActive ? " active" : ""}`}
        onClick={onSelectDevices}
        title="Devices — monitor and control your printers"
      >
        <span className="plate-tab-icon" aria-hidden>
          <svg width="13" height="13" viewBox="0 0 14 14" fill="none">
            <rect
              x="1.5"
              y="2.5"
              width="11"
              height="7"
              rx="1"
              stroke="currentColor"
              strokeWidth="1.1"
            />
            <path
              d="M4.5 11.5h5M7 9.5v2"
              stroke="currentColor"
              strokeWidth="1.1"
              strokeLinecap="round"
            />
          </svg>
        </span>
        <span className="plate-tab-name">Devices</span>
        <span className="plate-tab-meta">{deviceCount}</span>
      </button>
    </div>
  );
}
