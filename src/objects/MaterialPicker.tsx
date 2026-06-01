// Per-object material picker (OP-3) — a floating popover anchored under
// a row's material badge. Two views:
//   Assign — pick an existing project material; each row shows its slot
//            routing + the slot's filament colour/name.
//   Create — mint a new material routed to a chosen slot.
//
// "Material" is the existing concept (an object's 1-based extruder_id
// resolved to a slot via the plate's material_to_slot table) — the same
// the SlotBindingPanel Materials section manages. No new entity.

import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { SlotRef, FlatSlotOption } from "../printer/printerInstance";
import type { FilamentSummary } from "../material/filamentSummary";

export interface MaterialPickerProps {
  objectName: string;
  currentMaterial: number;
  /** Materials referenced on the plate (sorted ascending). */
  materials: number[];
  /** Next free material index for "create new". */
  nextMaterial: number;
  slots: FlatSlotOption[];
  materialToSlot: Record<number, SlotRef>;
  filamentByIdentity: Map<string, FilamentSummary>;
  /** Whether minting a new material makes sense — false when the plate
   *  has a single object (a new material would just orphan the old). */
  allowCreate: boolean;
  anchorRect: DOMRect;
  onAssign: (material: number) => void;
  onCreate: (material: number, slot: SlotRef) => void;
  onClose: () => void;
}

const MENU_W = 224;

export function MaterialPicker({
  objectName,
  currentMaterial,
  materials,
  nextMaterial,
  slots,
  materialToSlot,
  filamentByIdentity,
  allowCreate,
  anchorRect,
  onAssign,
  onCreate,
  onClose,
}: MaterialPickerProps) {
  const [creating, setCreating] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDoc = (e: MouseEvent): void => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  const slotFor = (m: number): FlatSlotOption | null => {
    const ref = materialToSlot[m];
    if (!ref) return null;
    return (
      slots.find(
        (s) => s.ref.extruder === ref.extruder && s.ref.slot === ref.slot,
      ) ?? null
    );
  };
  const filamentLabel = (slot: FlatSlotOption | null): string => {
    if (!slot?.filament_identity) return "unassigned";
    return (
      filamentByIdentity.get(slot.filament_identity)?.display_name ??
      slot.filament_identity
    );
  };
  // A slot shows its colour only when a filament is actually loaded
  // (`filament_identity`). A leftover/cached spool colour with no
  // identity — e.g. the Bambu external feed (no RFID) after unload —
  // reads as empty (hollow dashed orb), not a solid swatch.
  const swatch = (slot: FlatSlotOption | null): CSSProperties => {
    const color = slot?.filament_identity ? slot.color : null;
    return {
      background: color ?? "transparent",
      border: color ? "none" : "1px dashed var(--text-muted)",
    };
  };

  // Fixed position, clamped to the viewport; below the badge, flipped
  // above when it would overflow the bottom edge.
  const rowCount = creating ? slots.length : materials.length + 1;
  const estH = 36 + rowCount * 34;
  const style: CSSProperties = {
    position: "fixed",
    left: Math.max(8, Math.min(anchorRect.left, window.innerWidth - MENU_W - 8)),
    top:
      anchorRect.bottom + estH > window.innerHeight - 8
        ? Math.max(8, anchorRect.top - estH - 4)
        : anchorRect.bottom + 4,
  };

  return (
    <div
      ref={menuRef}
      className="material-picker"
      style={style}
      onClick={(e) => e.stopPropagation()}
    >
      {!creating ? (
        <>
          <div className="material-picker-title">Material · {objectName}</div>
          {materials.map((m) => {
            const slot = slotFor(m);
            return (
              <button
                key={m}
                className={`material-picker-row ${m === currentMaterial ? "active" : ""}`}
                onClick={() => {
                  onAssign(m);
                  onClose();
                }}
              >
                <span className="material-picker-name">
                  <span className="material-picker-swatch" style={swatch(slot)} />
                  <span className="material-picker-mid">M{m}</span>
                  <span className="material-picker-arrow">→</span>
                  <span className="material-picker-slot">{slot?.label || "—"}</span>
                </span>
                <span className="material-picker-detail">{filamentLabel(slot)}</span>
              </button>
            );
          })}
          {allowCreate && (
            <>
              <div className="material-picker-sep" />
              <button
                className="material-picker-row material-picker-new"
                onClick={() => setCreating(true)}
              >
                <span className="material-picker-name">+ New material</span>
                <span className="material-picker-detail">route to slot…</span>
              </button>
            </>
          )}
        </>
      ) : (
        <>
          <button
            className="material-picker-title material-picker-back"
            onClick={() => setCreating(false)}
          >
            ‹ New material M{nextMaterial} — route to slot
          </button>
          {slots.length === 0 && (
            <div className="material-picker-empty">No slots — no printer bound.</div>
          )}
          {slots.map((slot) => (
            <button
              key={`${slot.ref.extruder}:${slot.ref.slot}`}
              className="material-picker-row"
              onClick={() => {
                onCreate(nextMaterial, slot.ref);
                onClose();
              }}
            >
              <span className="material-picker-name">
                <span className="material-picker-swatch" style={swatch(slot)} />
                {slot.label}
              </span>
              <span className="material-picker-detail">{filamentLabel(slot)}</span>
            </button>
          ))}
        </>
      )}
    </div>
  );
}
