// Small dialog that copies a filament into a new custom one. Collects a new
// brand and/or filament type (both pre-filled from the source); on Save the
// caller clones the source and opens the new filament's settings editor.

import { useState } from "react";
import { ModalBackdrop, ModalCloseButton } from "../ui/Modal";
import type { FilamentSummary } from "./filamentSummary";

export interface CloneFilamentDialogProps {
  /** Filament being copied (seeds the inputs + the header). */
  source: FilamentSummary;
  /** Known material types, for the type field's datalist. */
  materials: readonly string[];
  /** Create the clone with these labels (blank = keep the source's). */
  onClone: (vendor: string | null, filamentType: string | null) => void;
  onClose: () => void;
}

export function CloneFilamentDialog({
  source,
  materials,
  onClone,
  onClose,
}: CloneFilamentDialogProps): React.JSX.Element {
  const [vendor, setVendor] = useState(source.vendor);
  const [filamentType, setFilamentType] = useState(source.base_type);

  // At least one field must be filled, and the result must differ from the
  // source — otherwise it'd just be an unlabeled duplicate.
  const v = vendor.trim();
  const t = filamentType.trim();
  const canSave =
    !!(v || t) && (v !== source.vendor || t !== source.base_type);

  const save = (): void => {
    if (!canSave) return;
    onClone(v || null, t || null);
  };

  return (
    <ModalBackdrop
      onDismiss={onClose}
      cardClassName="cfd-card"
      backdropClassName="cfd-backdrop"
      ariaLabelledBy="cfd-title"
    >
      <header className="psm-header">
        <div className="psm-header-mark" data-brand="filament">
          <span>🧵</span>
        </div>
        <div className="psm-header-text">
          <h2 id="cfd-title">Copy filament</h2>
          <p>New brand and/or type from “{source.display_name}”</p>
        </div>
        <ModalCloseButton onClick={onClose} />
      </header>

      <div className="cfd-body">
        <label className="cfd-field">
          <span>Brand</span>
          <input
            type="text"
            value={vendor}
            autoFocus
            onChange={(e) => setVendor(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && save()}
            placeholder={source.vendor}
          />
        </label>
        <label className="cfd-field">
          <span>Filament type</span>
          <input
            type="text"
            value={filamentType}
            list="cfd-materials"
            onChange={(e) => setFilamentType(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && save()}
            placeholder={source.base_type}
          />
          <datalist id="cfd-materials">
            {materials.map((m) => (
              <option key={m} value={m} />
            ))}
          </datalist>
        </label>
      </div>

      <footer className="cfd-foot">
        <button type="button" className="apm-btn" onClick={onClose}>
          Cancel
        </button>
        <button
          type="button"
          className="apm-btn primary"
          onClick={save}
          disabled={!canSave}
        >
          Create &amp; edit
        </button>
      </footer>
    </ModalBackdrop>
  );
}
