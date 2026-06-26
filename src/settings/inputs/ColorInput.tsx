// Color picker for libslic3r String options that hold `#RRGGBB`
// hex values (`filament_colour`, etc.).
//
// Native HTML5 color picker + a hex text field side by side. Either
// can drive the commit; both reflect each other.

import { useEffect, useState } from "react";
import { commitColor, isValidHexColor } from "./helpers";
import { defaultScalarFor, type OptionSummary } from "../types";

export interface ColorInputProps {
  schema: OptionSummary;
  value: string | null;
  onChange: (next: string) => void;
  disabled?: boolean;
}

const FALLBACK = "#888888";

export function ColorInput({
  schema,
  value,
  onChange,
  disabled = false,
}: ColorInputProps) {
  const fallback = defaultScalarFor(schema) ?? FALLBACK;
  const effective = value ?? fallback;
  const [draft, setDraft] = useState(effective);
  useEffect(() => setDraft(value ?? fallback), [value, fallback]);

  const safe = isValidHexColor(draft) ? draft : FALLBACK;

  const commitText = () => {
    const result = commitColor(draft);
    if (result.ok) {
      if (result.serialized !== effective.toLowerCase()) {
        onChange(result.serialized);
      }
      setDraft(result.serialized);
    } else {
      setDraft(effective);
    }
  };

  return (
    <div className="val-wrap val-color-wrap">
      <input
        type="color"
        className="val-color-swatch"
        value={safe}
        disabled={disabled}
        onChange={(e) => {
          const next = e.target.value.toLowerCase();
          setDraft(next);
          // Native color picker fires on every drag tick; we still
          // want one commit per change since the picker has its own
          // confirm UX. Commit immediately.
          if (next !== effective.toLowerCase()) onChange(next);
        }}
        aria-label={`${schema.label ?? schema.key} color picker`}
      />
      <input
        type="text"
        className="val-input val-color-hex"
        value={draft}
        disabled={disabled}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commitText}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        }}
        placeholder="#RRGGBB"
        spellCheck={false}
      />
    </div>
  );
}
