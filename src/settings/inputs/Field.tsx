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
  /** Winning cascade layer for this row (PR-4-7). Drives the
   *  authored-tier background tint + `--row-hue` CSS variable for
   *  the hover rule. Defaults to `"cascade"` (neutral) when not
   *  provided. */
  winningLayer?: CascadeLayer;
  /** Cascade ladder hover lifecycle (PR-4-8). Fired with the row's
   *  DOM node when the cursor enters/leaves; the panel mounts a
   *  single CascadeLadder portal centrally based on which row is
   *  currently hovered. */
  onRowEnter?: (el: HTMLElement) => void;
  onRowLeave?: () => void;
  /** SettingTooltip hover lifecycle (PR-4-11). Fired with the
   *  label's DOM node; distinct from the row enter/leave so the
   *  tooltip + cascade ladder can coexist without conflict. */
  onLabelEnter?: (el: HTMLElement) => void;
  onLabelLeave?: () => void;
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
  onLabelEnter,
  onLabelLeave,
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
    // (matches docs/design/styles.css:972-1029). Production CSS will
    // bind it in `:hover` once the design styles are formally
    // integrated; the Field already publishes the value so the
    // styling lift is a one-file change.
    ["--row-hue" as string]: String(LAYER_HUE[winningLayer]),
  };
  return (
    <div
      className={[
        "set-row",
        "px-3 py-2 flex items-center gap-2",
        authoredClass,
        tintClass,
        disabled ? "is-disabled opacity-60" : "",
        error ? "has-error" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      data-setting-id={schema.key}
      style={style}
      onMouseEnter={onRowEnter ? (e) => onRowEnter(e.currentTarget) : undefined}
      onMouseLeave={onRowLeave}
    >
      <div className="set-meta flex-1 min-w-0 flex items-center gap-2">
        <span
          className={`set-name truncate text-sm ${authored ? "font-semibold" : ""}`}
          title={schema.key}
          onMouseEnter={onLabelEnter ? (e) => onLabelEnter(e.currentTarget) : undefined}
          onMouseLeave={onLabelLeave}
        >
          {schema.label ?? schema.key}
        </span>
        {leadingBadge}
        {trailingBadge}
      </div>
      {resetButton}
      <div className="set-value">{children}</div>
      {error && (
        <div className="set-error w-full text-xs text-rose-500 mt-1" role="alert">
          {error}
        </div>
      )}
    </div>
  );
}
