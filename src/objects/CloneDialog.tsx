// Clone dialog: asks how many copies to make of the selected object(s),
// or to "fill the plate" (clone until the auto-arranger would need another
// plate). Confirms with a copy count (>= 1) or `null` for fill-plate.
//
// Pure-ish: it owns only the count input; the caller runs the clone command
// and closes it via onConfirm/onCancel.

import { useState } from "react";
import { ModalBackdrop, ModalCloseButton } from "../ui/Modal";

export interface CloneDialogProps {
  /** How many objects are being cloned — for the header label. */
  count: number;
  /** `copies` = a number (>= 1) for N copies, or `null` for "fill plate". */
  onConfirm: (copies: number | null) => void;
  onCancel: () => void;
}

/** Parse the copies field into a valid count, or null if not a usable
 *  integer >= 1. Exported for the self-check test. */
export function parseCopies(raw: string): number | null {
  const n = Number(raw);
  if (!Number.isInteger(n) || n < 1) return null;
  return n;
}

export function CloneDialog({
  count,
  onConfirm,
  onCancel,
}: CloneDialogProps): React.JSX.Element {
  const [raw, setRaw] = useState("1");
  const copies = parseCopies(raw);

  return (
    <ModalBackdrop
      onDismiss={onCancel}
      cardClassName="clone-dialog"
      role="dialog"
      ariaLabelledBy="clone-dialog-title"
    >
      <div className="apm-header">
        <div>
          <h2 id="clone-dialog-title">
            Clone {count} object{count === 1 ? "" : "s"}
          </h2>
          <p>Settings and grouping are copied with the geometry.</p>
        </div>
        <ModalCloseButton onClick={onCancel} />
      </div>
      <div className="clone-dialog-body">
        <label className="clone-dialog-row">
          <span>Copies</span>
          <input
            type="number"
            min={1}
            value={raw}
            autoFocus
            onChange={(e) => setRaw(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && copies !== null) onConfirm(copies);
            }}
          />
        </label>
        <p className="clone-dialog-hint">
          …or fill the plate with as many copies as fit.
        </p>
      </div>
      <div className="apm-actions clone-dialog-actions">
        <button type="button" className="apm-btn" onClick={onCancel}>
          Cancel
        </button>
        <button
          type="button"
          className="apm-btn"
          onClick={() => onConfirm(null)}
        >
          Fill plate
        </button>
        <button
          type="button"
          className="apm-btn primary"
          disabled={copies === null}
          onClick={() => copies !== null && onConfirm(copies)}
        >
          Clone
        </button>
      </div>
    </ModalBackdrop>
  );
}
