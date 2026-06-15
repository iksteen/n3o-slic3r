// "Send to plate" picker — a floating popover anchored to the selection
// bar's "Send to" button. Lists the other plates as move targets (each
// keeps the objects' authored XYZ), plus a "New plate" option that
// creates a plate bound to the same printer and moves there.
//
// Reuses the `material-picker` visual language (same floating popover,
// rows, separator) so the two pickers read as siblings.

import { useRef } from "react";
import { usePopoverDismiss } from "../ui/usePopoverDismiss";
import { popoverPosition } from "../ui/popoverPosition";
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
  usePopoverDismiss(menuRef, onClose);

  const targets = plates.filter((p) => p.id !== currentPlateId);

  const rowCount = targets.length + 1;
  const estH = 36 + rowCount * 34;
  const style = popoverPosition(anchorRect, MENU_W, estH);

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
