// Numeric input for libslic3r `Float` / `Int` options (PR-4-2).
//
// Matches the mockup's `.val-wrap` + `.val-input` + `.val-unit`
// pattern (docs/design/SettingsPanel.jsx:236-249). Commits on blur
// and on Enter, NOT per keystroke (typing should not constantly
// fire cascade re-resolves).

import { useEffect, useState } from "react";
import { commitNumber, formatNumber } from "./helpers";
import { defaultScalarFor, type OptionSummary } from "../types";

export interface NumberInputProps {
  schema: OptionSummary;
  /** Serialized libslic3r value or null. When null, falls back to
   *  the schema's `default_value`. */
  value: string | null;
  onChange: (next: string) => void;
  disabled?: boolean;
  /** Optional sidetext suffix (mm, °C, %, etc.). Falls back to a
   *  schema-derived hint when not provided; PR-4-1's
   *  `OptionSummary` doesn't carry sidetext yet, so the panel
   *  passes it explicitly until that lands. */
  unit?: string | null;
  /** Numeric bounds — passed through from `OptionSummary.min/max`
   *  once those fields surface; PR-4-2 ships the prop but the
   *  current schema layer doesn't propagate them. */
  min?: number;
  max?: number;
  step?: number;
}

export function NumberInput({
  schema,
  value,
  onChange,
  disabled = false,
  unit,
  min,
  max,
  step,
}: NumberInputProps) {
  // Local draft state so typing is responsive (no per-keystroke
  // commit). Reset when the parent's value changes (e.g. cascade
  // re-resolve after a printer switch).
  const fallback = defaultScalarFor(schema) ?? "";
  const initial = value ?? fallback;
  const [draft, setDraft] = useState(initial);
  useEffect(() => {
    setDraft(value ?? fallback);
  }, [value, fallback]);

  const commit = () => {
    const result = commitNumber(draft, { min, max, step });
    if (result.ok) {
      // Normalize draft to the formatted value so the input visually
      // settles after commit.
      setDraft(formatNumber(result.value));
      // Only fire onChange when the serialized form actually differs
      // from what the parent passed — avoids spurious cascade
      // round-trips when the user blurs without editing.
      if (result.serialized !== (value ?? fallback)) {
        onChange(result.serialized);
      }
    } else {
      // Parse failure → revert to last good value.
      setDraft(value ?? fallback);
    }
  };

  return (
    <div className="val-wrap">
      <input
        className="val-input"
        type="number"
        value={draft}
        disabled={disabled}
        step={step ?? "any"}
        min={min}
        max={max}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            (e.target as HTMLInputElement).blur();
          }
        }}
      />
      {unit && <span className="val-unit">{unit}</span>}
    </div>
  );
}

