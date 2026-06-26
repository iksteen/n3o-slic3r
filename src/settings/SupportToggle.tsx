// Per-object support toggle — FR-3D-6.
//
// Single on/off switch on the selected object. Maps to libslic3r's
// `enable_support` bool, stored as an Object-tier override.
// Visible only when the Object tab is active and an object is
// selected; otherwise the host elides this component.

import { parseBool, formatBool } from "./inputs/helpers";

export interface SupportToggleProps {
  /** Current value of `enable_support` for the selected object.
   *  When null, falls back to the cascade resolution (typically
   *  off for hobbyist defaults). */
  value: string | null;
  onChange: (next: string) => void;
  disabled?: boolean;
}

export function SupportToggle({
  value,
  onChange,
  disabled = false,
}: SupportToggleProps) {
  const on = parseBool(value ?? "0") ?? false;
  return (
    <div className="support-toggle flex items-center gap-3 px-3 py-2 rounded bg-neutral-100 dark:bg-neutral-800 border border-neutral-200 dark:border-neutral-700">
      <button
        type="button"
        role="switch"
        aria-checked={on}
        aria-label="Auto-generate supports"
        disabled={disabled}
        className={`val-toggle inline-block w-9 h-5 rounded-full transition-colors ${
          on ? "bg-emerald-600" : "bg-neutral-400 dark:bg-neutral-600"
        } ${disabled ? "opacity-40" : "cursor-pointer"}`}
        onClick={() => onChange(formatBool(!on))}
      >
        <span
          className={`block w-4 h-4 rounded-full bg-white shadow transition-transform ${
            on ? "translate-x-4" : "translate-x-0.5"
          }`}
          style={{ marginTop: 2 }}
        />
      </button>
      <div className="support-toggle-text text-sm">
        <div className="font-medium">
          {on ? "Auto-generated supports" : "No supports"}
        </div>
        <div className="text-xs text-neutral-600 dark:text-neutral-400">
          {on
            ? "Uses the current cascade settings for support geometry."
            : "Overhangs may sag or fail without supports."}
        </div>
      </div>
    </div>
  );
}
