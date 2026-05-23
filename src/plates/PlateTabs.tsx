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

export function PlateTabs() {
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

  return (
    <div className="plate-tabs" role="tablist" aria-label="Plates">
      <div className="plate-tabs-scroll">
        {plates.map((plate) => {
          const isActive = plate.id === activePlateId;
          const isEditing = editingId === plate.id;
          return (
            <div
              key={plate.id}
              role="tab"
              aria-selected={isActive}
              className={`plate-tab${isActive ? " active" : ""}`}
              onClick={() => {
                if (isEditing) return;
                if (plate.id === activePlateId) return;
                void setActivePlate(plate.id);
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
      <button
        type="button"
        className="plate-tab-add"
        onClick={() => {
          void addPlate(null).catch((err) => {
            console.error("[plates] addPlate failed", err);
          });
        }}
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
        <span>New plate</span>
      </button>
    </div>
  );
}
