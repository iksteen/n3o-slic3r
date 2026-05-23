// On/off toggle for libslic3r `bool` options (PR-4-2).
//
// Matches the mockup's `.val-toggle` pattern (docs/design/
// SettingsPanel.jsx:222-227). Commits on click.

import { parseBool, formatBool } from "./helpers";
import type { OptionSummary } from "../types";

export interface BoolInputProps {
  schema: OptionSummary;
  /** Serialized libslic3r value ("1" / "0") or null for the
   *  cascade's resolved value (which the panel layer reads from
   *  `default_value` when no override is set). */
  value: string | null;
  onChange: (next: string) => void;
  disabled?: boolean;
}

export function BoolInput({
  schema,
  value,
  onChange,
  disabled = false,
}: BoolInputProps) {
  const effective =
    parseBool(value ?? schema.default_value ?? "0") ?? false;
  return (
    <div className="val-toggle-wrap">
      <button
        type="button"
        role="switch"
        aria-checked={effective}
        aria-label={schema.label ?? schema.key}
        disabled={disabled}
        className={`val-toggle${effective ? " on" : ""}${
          disabled ? " is-disabled" : ""
        }`}
        onClick={() => onChange(formatBool(!effective))}
      />
    </div>
  );
}
