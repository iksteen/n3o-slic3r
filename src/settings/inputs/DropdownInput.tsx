// Native <select> for libslic3r `Enum` options.
//
// Matches the mockup's `.val-select` pattern (docs/dev/design/
// SettingsPanel.jsx:229-234). Commits on change.
//
// Takes the `[key, label]` option list as a prop; the panel plumbs
// it from `OptionSummary.enum_values` (sourced from the FFI's
// `OptionDef::enum_values`).

import { useEffect, useState } from "react";
import { defaultScalarFor, type OptionSummary } from "../types";

export interface DropdownInputProps {
  schema: OptionSummary;
  value: string | null;
  onChange: (next: string) => void;
  disabled?: boolean;
  /** `[key, label]` pairs in libslic3r declaration order. The
   *  serialized wire value is the `key`; `label` is what the user
   *  sees. */
  options: ReadonlyArray<readonly [string, string]>;
}

export function DropdownInput({
  schema,
  value,
  onChange,
  disabled = false,
  options,
}: DropdownInputProps) {
  const effective =
    value ?? defaultScalarFor(schema) ?? options[0]?.[0] ?? "";
  // Local mirror so we can show the user's choice immediately even
  // before the parent re-resolves the cascade.
  const [local, setLocal] = useState(effective);
  useEffect(() => setLocal(effective), [effective]);

  return (
    <select
      className="val-select"
      value={local}
      disabled={disabled}
      onChange={(e) => {
        const next = e.target.value;
        setLocal(next);
        if (next !== effective) onChange(next);
      }}
      aria-label={schema.label ?? schema.key}
    >
      {options.map(([key, label]) => (
        <option key={key} value={key}>
          {label || key}
        </option>
      ))}
    </select>
  );
}
