// Pre-send options dialog — the per-print toggles (bed leveling,
// flow / vibration calibration, timelapse) shown when Send is clicked
// on a printer whose driver supports them (Bambu, Snapmaker U1).
//
// The toggles are sticky per printer instance: the dialog seeds from the
// instance's persisted `send_options` and persists them on Send (not on
// Cancel), so the next send re-opens with the last-used values. Labels
// track the vendor's own terminology for the same underlying option
// (Bambu "vibration calibration" ⇔ U1 "input shaper calibration").

import { useState } from "react";
import { ModalBackdrop } from "../ui/Modal";
import type { SendOptions } from "../printer/printerInstance";

export interface SendOptionsDialogProps {
  /** Driver kind — picks the vendor-specific option labels. The caller
   *  only opens this dialog for kinds that support send options. */
  kind: "bambu" | "u1";
  /** The instance's persisted options, seeding the checkboxes. */
  initial: SendOptions;
  /** Print + persist with the (possibly edited) options. */
  onSend: (options: SendOptions) => void;
  onCancel: () => void;
}

/** One labeled checkbox row. */
function OptionRow({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}): React.JSX.Element {
  return (
    <label className="send-opt-row" title={hint}>
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>{label}</span>
    </label>
  );
}

export function SendOptionsDialog({
  kind,
  initial,
  onSend,
  onCancel,
}: SendOptionsDialogProps): React.JSX.Element {
  const [options, setOptions] = useState<SendOptions>(initial);
  const set = (patch: Partial<SendOptions>): void =>
    setOptions((prev) => ({ ...prev, ...patch }));

  return (
    <ModalBackdrop
      onDismiss={onCancel}
      cardClassName="psm-discard-card send-options-card"
      ariaLabelledBy="send-options-title"
    >
      <h3 id="send-options-title" className="psm-discard-title">
        Print options
      </h3>
      <div className="send-opt-list">
        <OptionRow
          label="Auto bed leveling"
          hint="Check the heatbed's flatness before printing."
          checked={options.bed_leveling}
          onChange={(v) => set({ bed_leveling: v })}
        />
        <OptionRow
          label={
            kind === "bambu"
              ? "Flow dynamics calibration"
              : "Flow calibration"
          }
          hint="Calibrate dynamic flow / pressure advance before printing."
          checked={options.flow_calibration}
          onChange={(v) => set({ flow_calibration: v })}
        />
        <OptionRow
          label={
            kind === "bambu"
              ? "Vibration calibration"
              : "Input shaper calibration"
          }
          hint="Calibrate vibration compensation before printing."
          checked={options.vibration_calibration}
          onChange={(v) => set({ vibration_calibration: v })}
        />
        <OptionRow
          label="Timelapse"
          hint="Record a timelapse of the print with the built-in camera."
          checked={options.timelapse}
          onChange={(v) => set({ timelapse: v })}
        />
      </div>
      <div className="send-opt-actions">
        <button type="button" className="tb-btn" onClick={onCancel}>
          Cancel
        </button>
        <button
          type="button"
          className="tb-btn primary"
          onClick={() => onSend(options)}
        >
          Send
        </button>
      </div>
    </ModalBackdrop>
  );
}
