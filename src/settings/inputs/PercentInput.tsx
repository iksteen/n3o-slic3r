// Percent input for libslic3r `Percent` / `FloatOrPercent` options.
//
// For pure `Percent`, this is a NumberInput-with-% kind of widget.
// For `FloatOrPercent`, it adds a small absolute-vs-percent toggle
// (mm vs %) so the user can switch between authoring `0.5` (mm)
// and `120%` (of nozzle diameter / extrusion width).

import { useEffect, useState } from "react";
import {
  commitFloatOrPercent,
  commitPercent,
  formatFloatOrPercent,
  formatNumber,
  parseFloatOrPercent,
} from "./helpers";
import { defaultScalarFor, optionTypeKind, type OptionSummary } from "../types";

export interface PercentInputProps {
  schema: OptionSummary;
  value: string | null;
  onChange: (next: string) => void;
  disabled?: boolean;
  min?: number;
  max?: number;
  step?: number;
}

export function PercentInput({
  schema,
  value,
  onChange,
  disabled = false,
  min,
  max,
  step,
}: PercentInputProps) {
  const kind = optionTypeKind(schema);
  const allowsAbsolute = kind === "float-or-percent";

  // Local draft + percent-mode flag, mirroring the wire form
  // ("75" vs "75%"). For pure Percent, the flag is always true.
  const fallback = defaultScalarFor(schema) ?? "";
  const initial = value ?? fallback;
  const [draft, setDraft] = useState(initial);
  useEffect(() => setDraft(value ?? fallback), [value, fallback]);

  const parsed = parseFloatOrPercent(initial);
  const initialPercent = allowsAbsolute ? parsed?.percent ?? true : true;
  const [percent, setPercent] = useState<boolean>(initialPercent);
  useEffect(() => {
    const p = parseFloatOrPercent(value ?? fallback);
    if (p) setPercent(allowsAbsolute ? p.percent : true);
  }, [value, fallback, allowsAbsolute]);

  const commit = (overridePercent?: boolean) => {
    const wantPercent = overridePercent ?? percent;
    if (allowsAbsolute) {
      const result = commitFloatOrPercent(
        // Make sure the suffix matches the chosen mode so commit
        // round-trips back through `parseFloatOrPercent`.
        wantPercent ? withPercent(draft) : withoutPercent(draft),
        { min, max, step },
      );
      if (result.ok) {
        setDraft(formatFloatOrPercent(result.value));
        if (result.serialized !== (value ?? fallback)) {
          onChange(result.serialized);
        }
      } else {
        setDraft(value ?? fallback);
      }
    } else {
      const result = commitPercent(draft, { min, max, step });
      if (result.ok) {
        setDraft(formatNumber(result.value));
        if (result.serialized !== (value ?? fallback)) {
          onChange(result.serialized);
        }
      } else {
        setDraft(value ?? fallback);
      }
    }
  };

  return (
    <div className="val-wrap val-percent-wrap">
      <input
        className="val-input"
        type="text"
        inputMode="decimal"
        value={withoutPercent(draft)}
        disabled={disabled}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => commit()}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        }}
      />
      {allowsAbsolute ? (
        <button
          type="button"
          className={`val-unit val-unit-toggle${percent ? " is-percent" : ""}`}
          disabled={disabled}
          aria-label={percent ? "Switch to absolute" : "Switch to percent"}
          onClick={() => {
            const next = !percent;
            setPercent(next);
            commit(next);
          }}
        >
          {percent ? "%" : "mm"}
        </button>
      ) : (
        <span className="val-unit">%</span>
      )}
    </div>
  );
}

function withoutPercent(s: string): string {
  return s.endsWith("%") ? s.slice(0, -1) : s;
}

function withPercent(s: string): string {
  const stripped = withoutPercent(s);
  return stripped === "" ? "" : `${stripped}%`;
}
