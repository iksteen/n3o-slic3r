// Per-object material picker — a floating popover anchored under a row's
// material badge. Two views:
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
import { slotForMaterial, slotColor, swatchStyle } from "../material/materials";

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
  // Latest onClose via a ref so the document listeners attach once on
  // mount rather than re-subscribing on every render (onClose is an
  // inline arrow from the parent).
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    const onDoc = (e: MouseEvent): void => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onCloseRef.current();
      }
    };
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") onCloseRef.current();
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, []);

  const slotFor = (m: number): FlatSlotOption | null =>
    slotForMaterial(m, materialToSlot, slots);
  const filamentLabel = (slot: FlatSlotOption | null): string => {
    if (!slot?.filament_identity) return "unassigned";
    return (
      filamentByIdentity.get(slot.filament_identity)?.display_name ??
      slot.filament_identity
    );
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
                  <span className="material-picker-swatch" style={swatchStyle(slotColor(slot))} />
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
                <span className="material-picker-swatch" style={swatchStyle(slotColor(slot))} />
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
