// "Send to plate" picker — a floating popover anchored to the selection
// bar's "Send to" button. Lists the other plates as move targets (each
// keeps the objects' authored XYZ), plus a "New plate" option that
// creates a plate bound to the same printer and moves there.
//
// Reuses the `material-picker` visual language (same floating popover,
// rows, separator) so the two pickers read as siblings.

import { useEffect, useRef, type CSSProperties } from "react";
import type { PlateId } from "../viewport/types";
import type { PlateTabView } from "../plates/usePlateTabs";

export interface SendToPlatePickerProps {
  /** How many objects are selected (for the title). */
  count: number;
  plates: PlateTabView[];
  /** The plate the objects are on — excluded from the target list. */
  currentPlateId: PlateId;
  anchorRect: DOMRect;
  onSend: (toPlateId: PlateId) => void;
  onSendNew: () => void;
  onClose: () => void;
}

const MENU_W = 224;

export function SendToPlatePicker({
  count,
  plates,
  currentPlateId,
  anchorRect,
  onSend,
  onSendNew,
  onClose,
}: SendToPlatePickerProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  // Latest onClose via a ref so the document listeners attach once on
  // mount rather than re-subscribing every render.
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

  const targets = plates.filter((p) => p.id !== currentPlateId);

  // Fixed position, clamped to the viewport; below the button, flipped
  // above when it would overflow the bottom edge.
  const rowCount = targets.length + 1;
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
      <div className="material-picker-title">
        Send {count} {count === 1 ? "object" : "objects"} to…
      </div>
      {targets.map((p) => (
        <button
          key={p.id}
          className="material-picker-row"
          onClick={() => {
            onSend(p.id);
            onClose();
          }}
        >
          <span className="material-picker-name">{p.name}</span>
          <span className="material-picker-detail">
            {p.printerLabel ?? "no printer"} ·{" "}
            {p.objectCount} {p.objectCount === 1 ? "obj" : "objs"}
          </span>
        </button>
      ))}
      {targets.length > 0 && <div className="material-picker-sep" />}
      <button
        className="material-picker-row material-picker-new"
        onClick={() => {
          onSendNew();
          onClose();
        }}
      >
        <span className="material-picker-name">+ New plate</span>
        <span className="material-picker-detail">same printer</span>
      </button>
    </div>
  );
}
