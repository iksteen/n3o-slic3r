// Type-dispatched input control for one option row, shared by the
// machine-settings panel. Branches on the option's libslic3r type:
//   - multiline (G-code `coStrings` with the multiline hint) → a pop-out
//     editor (read-only preview + "Edit" opening a textarea modal), when
//     `multilineEditable`; otherwise a read-only textarea.
//   - other vector kinds → read-only text (per-extruder editors are a
//     follow-up; surfaced so the value is visible, not corruptible).
//   - scalars → the matching scalar input via `renderScalarInput`.

import { useState } from "react";
import { renderScalarInput } from "../SettingsPanel";
import { isVectorKind, optionTypeKind, type OptionSummary } from "../types";
import { ModalBackdrop, ModalCloseButton } from "../../ui/Modal";

export interface SettingControlProps {
  schema: OptionSummary;
  /** Serialized value to show (override if set, else resolved base). */
  value: string | null;
  onChange: (next: string) => void;
  disabled?: boolean;
  /** When true, multiline G-code fields get the pop-out editor instead
   *  of a read-only textarea. */
  multilineEditable?: boolean;
}

export function SettingControl({
  schema,
  value,
  onChange,
  disabled = false,
  multilineEditable = false,
}: SettingControlProps): React.JSX.Element {
  const kind = optionTypeKind(schema);

  // `multiline` covers both scalar `String` G-code fields (machine_*_gcode)
  // and multiline vector `Strings` — the type alone doesn't distinguish them.
  if (schema.multiline) {
    return multilineEditable && !disabled ? (
      <MultilineEditor schema={schema} value={value} onChange={onChange} />
    ) : (
      <textarea
        className="val-input val-input-multiline"
        value={value ?? ""}
        disabled
        readOnly
        rows={Math.min(10, Math.max(2, value?.split("\n").length ?? 1))}
      />
    );
  }

  if (isVectorKind(kind)) {
    // Per-extruder vector editing is a follow-up; show read-only so the
    // value is visible without a path to corrupt it.
    return (
      <input
        className="val-input val-input-fallback"
        type="text"
        value={value ?? ""}
        readOnly
        disabled
      />
    );
  }

  return renderScalarInput(kind, schema, value, onChange, disabled);
}

/** Read-only G-code preview + an "Edit" button that opens a textarea
 *  pop-out. Commits the edited text on Save. Exported so the filament
 *  editor (whose fields are vectors, so it can't use `SettingControl`
 *  wholesale) can reuse the same pop-out for its multiline fields. */
export function MultilineEditor({
  schema,
  value,
  onChange,
}: {
  schema: OptionSummary;
  value: string | null;
  onChange: (next: string) => void;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const current = value ?? "";
  const lines = current.trim() === "" ? 0 : current.split("\n").length;

  return (
    <div className="mc-gcode">
      <button
        type="button"
        className="mc-gcode-btn"
        onClick={() => {
          setDraft(current);
          setOpen(true);
        }}
      >
        <span className={`mc-gcode-status${lines === 0 ? " empty" : ""}`}>
          {lines === 0 ? "empty" : `${lines} ${lines === 1 ? "line" : "lines"}`}
        </span>
        <span className="mc-gcode-pencil" aria-hidden>
          ✎
        </span>
        <span className="mc-gcode-edit-label">Edit…</span>
      </button>
      {open && (
        <ModalBackdrop
          onDismiss={() => setOpen(false)}
          cardClassName="mc-gcode-modal"
          ariaLabelledBy="mc-gcode-title"
        >
          <header className="mc-gcode-modal-header">
            <h3 id="mc-gcode-title">{schema.label ?? schema.key}</h3>
            <ModalCloseButton onClick={() => setOpen(false)} />
          </header>
          <textarea
            className="mc-gcode-modal-area"
            value={draft}
            spellCheck={false}
            autoFocus
            onChange={(e) => setDraft(e.target.value)}
          />
          <footer className="mc-gcode-modal-footer">
            <button type="button" onClick={() => setOpen(false)}>
              Cancel
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => {
                onChange(draft);
                setOpen(false);
              }}
            >
              Save
            </button>
          </footer>
        </ModalBackdrop>
      )}
    </div>
  );
}
