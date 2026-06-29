// Small dialog that names a new custom quality profile. The caller duplicates
// the current profile under the entered name. Reuses the generic mini-dialog
// styles (`cfd-*`) the filament copy dialog defines.

import { useState } from "react";
import { ModalBackdrop, ModalCloseButton } from "../ui/Modal";

export interface QualityProfileNameDialogProps {
  /** Profile being duplicated (seeds the placeholder + header). */
  sourceName: string;
  onCreate: (name: string) => void;
  onClose: () => void;
}

export function QualityProfileNameDialog({
  sourceName,
  onCreate,
  onClose,
}: QualityProfileNameDialogProps): React.JSX.Element {
  const [name, setName] = useState("");
  const trimmed = name.trim();
  const canCreate = trimmed.length > 0;

  const create = (): void => {
    if (canCreate) onCreate(trimmed);
  };

  return (
    <ModalBackdrop
      onDismiss={onClose}
      cardClassName="cfd-card"
      backdropClassName="cfd-backdrop"
      ariaLabelledBy="qnd-title"
    >
      <header className="psm-header">
        <div className="psm-header-mark" data-brand="filament">
          <span>🎚️</span>
        </div>
        <div className="psm-header-text">
          <h2 id="qnd-title">Save as custom profile</h2>
          <p>A copy of “{sourceName}” with your current quality settings</p>
        </div>
        <ModalCloseButton onClick={onClose} />
      </header>

      <div className="cfd-body">
        <label className="cfd-field">
          <span>Profile name</span>
          <input
            type="text"
            value={name}
            autoFocus
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && create()}
            placeholder={`${sourceName} (custom)`}
          />
        </label>
      </div>

      <footer className="cfd-foot">
        <button type="button" className="apm-btn" onClick={onClose}>
          Cancel
        </button>
        <button
          type="button"
          className="apm-btn primary"
          onClick={create}
          disabled={!canCreate}
        >
          Create
        </button>
      </footer>
    </ModalBackdrop>
  );
}
