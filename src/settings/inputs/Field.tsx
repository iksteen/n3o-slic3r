// Shared row wrapper for every settings input (PR-4-2).
//
// Reserves the layout slots the rest of Phase 4 will fill:
//
//   ┌─────────────────────────────────────────────────────────┐
//   │ <label>                                            <reset>│   ← PR-4-9
//   │ <breadcrumb / rule / chips slot>          <input> <suffix>│
//   │                                              [error msg] │   ← PR-4-11
//   │ <badges slot — objects-overrides, hidden, scope>        │   ← PR-4-9, PR-4-5
//   └─────────────────────────────────────────────────────────┘
//
// The actual input control (NumberInput etc.) renders into the
// `children` slot. The wrapper itself is presentation-only — it
// doesn't manage value state.

import type { ReactNode } from "react";
import type { OptionSummary } from "../types";

export interface FieldProps {
  schema: OptionSummary;
  /** Serialized libslic3r value. `null` means "no override at this
   *  tier; the cascade's resolved value is what's actually shown
   *  in the input control." */
  value: string | null;
  /** Commit a new serialized value. The wrapper itself never
   *  invokes this; the child input does, on blur/Enter/change. */
  onChange: (next: string) => void;
  /** Disables the input. Used by PR-4-9 to gray out project-scope
   *  rows on the Object tab. */
  disabled?: boolean;
  /** Validation error from PR-4-11 (`slicer_validate_option` round-
   *  trip) or from a child input's own commit failure. Renders
   *  below the input in `.set-error`. */
  error?: string | null;
  /** Slots reserved for later tickets — wrapper renders them in
   *  fixed positions so adding badges later doesn't reflow the
   *  row. PR-4-2 ships the wrapper with these all empty. */
  leadingBadge?: ReactNode;
  trailingBadge?: ReactNode;
  /** Right-aligned reset affordance (PR-4-9). Empty here. */
  resetButton?: ReactNode;
  /** The input control. */
  children: ReactNode;
}

export function Field({
  schema,
  disabled = false,
  error,
  leadingBadge,
  trailingBadge,
  resetButton,
  children,
}: FieldProps) {
  return (
    <div
      className={`set-row${disabled ? " is-disabled" : ""}${error ? " has-error" : ""}`}
      data-setting-id={schema.key}
    >
      <div className="set-meta">
        <span className="set-name" title={schema.key}>
          {schema.label ?? schema.key}
        </span>
        {leadingBadge}
        {trailingBadge}
      </div>
      {resetButton}
      <div className="set-value">{children}</div>
      {error && (
        <div className="set-error" role="alert">
          {error}
        </div>
      )}
    </div>
  );
}
