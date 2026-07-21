// A single settings row holding two (or more) related scalar inputs side by
// side — the dual-entry layout shared by the machine panel's Orca multi-option
// lines ("Resonance Avoidance Speed" → Min/Max) and the extruder panel's
// two-member optgroups ("Layer height limits" → Min/Max). Reuses the coPoint
// `.point-input` look. The members are independent keys, so value/override
// access is supplied by the caller (scalar map vs per-extruder vector index).

import { Field, NumberInput } from "../settings/inputs";
import type { PrinterAwareOptionSummary } from "../settings/types";
import { shortUnit } from "../settings/units";

export interface PairedInputRowProps {
  /** Shared row label — the Orca line label or the optgroup name. */
  label: string;
  members: PrinterAwareOptionSummary[];
  /** Serialized value for a member (override if set, else resolved). */
  valueOf: (schema: PrinterAwareOptionSummary) => string | null;
  overriddenOf: (schema: PrinterAwareOptionSummary) => boolean;
  onSet: (schema: PrinterAwareOptionSummary, next: string) => void;
  onClear: (schema: PrinterAwareOptionSummary) => void;
}

export function PairedInputRow({
  label,
  members,
  valueOf,
  overriddenOf,
  onSet,
  onClear,
}: PairedInputRowProps): React.JSX.Element {
  const anyOverridden = members.some(overriddenOf);
  return (
    <Field
      // Synthetic schema: the row's name is the shared label, not a key's.
      schema={{ ...members[0], key: `line:${label}`, label }}
      value={null}
      onChange={() => {}}
      resetButton={
        anyOverridden ? (
          <button
            type="button"
            className="reset-btn"
            title="Reset to printer default"
            aria-label={`Reset ${label}`}
            onClick={() => members.forEach((m) => overriddenOf(m) && onClear(m))}
          >
            ↺
          </button>
        ) : null
      }
      winningLayer={anyOverridden ? "user" : "cascade"}
    >
      <div className="point-input">
        {members.map((m) => (
          <label className="point-axis" key={m.key}>
            <span>{m.label ?? m.key}</span>
            <NumberInput
              schema={m}
              value={valueOf(m)}
              onChange={(next) => onSet(m, next)}
            />
          </label>
        ))}
        {members[0].sidetext && (
          <span className="val-unit" title={members[0].sidetext}>
            {shortUnit(members[0].sidetext)}
          </span>
        )}
      </div>
    </Field>
  );
}
