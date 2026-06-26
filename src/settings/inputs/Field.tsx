// Shared row wrapper for every settings input.
//
// Lays out the row's slots:
//
//   ┌─────────────────────────────────────────────────────────┐
//   │ <label>                                            <reset>│
//   │ <breadcrumb / rule / chips slot>          <input> <suffix>│
//   │                                              [error msg] │
//   │ <badges slot — objects-overrides, hidden, scope>        │
//   └─────────────────────────────────────────────────────────┘
//
// The actual input control (NumberInput etc.) renders into the
// `children` slot. The wrapper itself is presentation-only — it
// doesn't manage value state.

import type { CSSProperties, ReactNode } from "react";
import type { OptionSummary } from "../types";
import {
  LAYER_HUE,
  LAYER_TINT_CLASS,
  isAuthoredTier,
  type CascadeLayer,
} from "../layers";

export interface FieldProps {
  schema: OptionSummary;
  /** Serialized libslic3r value. `null` means "no override at this
   *  tier; the cascade's resolved value is what's actually shown
   *  in the input control." */
  value: string | null;
  /** Commit a new serialized value. The wrapper itself never
   *  invokes this; the child input does, on blur/Enter/change. */
  onChange: (next: string) => void;
  /** Disables the input. Used to gray out project-scope
   *  rows on the Object tab. */
  disabled?: boolean;
  /** Validation error (`slicer_validate_option` round-trip) or from
   *  a child input's own commit failure. Renders below the input
   *  in `.set-error`. */
  error?: string | null;
  /** Badge slots — rendered in fixed positions so they don't
   *  reflow the row when present. */
  leadingBadge?: ReactNode;
  trailingBadge?: ReactNode;
  /** Right-aligned reset affordance. */
  resetButton?: ReactNode;
  /** Winning cascade layer for this row. Drives the
   *  authored-tier background tint + `--row-hue` CSS variable for
   *  the hover rule. Defaults to `"cascade"` (neutral) when not
   *  provided. */
  winningLayer?: CascadeLayer;
  /** Cascade ladder hover lifecycle. Fired with the row's
   *  DOM node when the cursor enters/leaves; the panel mounts a
   *  single CascadeLadder portal centrally based on which row is
   *  currently hovered. The ladder also carries the row's
   *  description/tip (the separate tooltip is folded in), so there
   *  is no separate label-hover lifecycle. */
  onRowEnter?: (el: HTMLElement) => void;
  onRowLeave?: () => void;
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
  winningLayer = "cascade",
  onRowEnter,
  onRowLeave,
  children,
}: FieldProps) {
  const authored = isAuthoredTier(winningLayer);
  const tintClass = LAYER_TINT_CLASS[winningLayer] ?? "";
  const authoredClass = authored
    ? winningLayer === "object"
      ? "authored-object"
      : winningLayer === "project"
        ? "authored-project"
        : "authored-user"
    : "";
  const style: CSSProperties = {
    // The `--row-hue` CSS var feeds the source-rule hover treatment
    // (matches docs/dev/design/styles.css:972-1029). Production CSS will
    // bind it in `:hover` once the design styles are formally
    // integrated; the Field already publishes the value so the
    // styling lift is a one-file change.
    ["--row-hue" as string]: String(LAYER_HUE[winningLayer]),
  };
  return (
    <div
      className={[
        "set-row",
        authoredClass,
        tintClass,
        disabled ? "is-disabled" : "",
        error ? "has-error" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      data-setting-id={schema.key}
      style={{ ...style, ...(disabled ? { opacity: 0.6 } : {}) }}
      onMouseEnter={onRowEnter ? (e) => onRowEnter(e.currentTarget) : undefined}
      onMouseLeave={onRowLeave}
    >
      <div className="set-meta">
        <span className="set-name" data-key={schema.key}>
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
