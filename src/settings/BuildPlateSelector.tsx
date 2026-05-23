// Build plate selector (PR-4-5) — FR-CAS-9.
//
// Dropdown of the active printer's `supported_build_plates`. The
// value drives the `BuildPlate` cascade layer the next
// `cascade_resolve` call resolves against (the host wires this into
// its ContextJson; Phase 5's project model owns durable storage).
//
// When the printer carries a default plate (`printerDefault`) and
// the user hasn't overridden, the selector shows a `printer default`
// badge. The selection clears the badge; setting the value back to
// the default restores it.

import { useEffect, useState } from "react";

export interface BuildPlateSelectorProps {
  /** All plate identities this printer supports. */
  plates: readonly string[];
  /** Currently selected plate identity. */
  value: string;
  onChange: (next: string) => void;
  /** Plate identity the printer profile considers its default.
   *  When the user's selection matches, the `printer default` badge
   *  renders. */
  printerDefault?: string | null;
  disabled?: boolean;
}

export function BuildPlateSelector({
  plates,
  value,
  onChange,
  printerDefault = null,
  disabled = false,
}: BuildPlateSelectorProps) {
  // Local mirror so the dropdown re-renders crisply during the
  // commit round-trip back through the cascade resolve.
  const [local, setLocal] = useState(value);
  useEffect(() => setLocal(value), [value]);

  const isDefault = printerDefault != null && local === printerDefault;

  return (
    <div className="config-chip config-chip-plate" data-default={isDefault}>
      <span className="config-chip-label">Plate</span>
      <select
        className="config-chip-select"
        value={local}
        disabled={disabled}
        onChange={(e) => {
          const next = e.target.value;
          setLocal(next);
          if (next !== value) onChange(next);
        }}
        aria-label="Build plate"
      >
        {plates.map((p) => (
          <option key={p} value={p}>
            {p}
          </option>
        ))}
      </select>
      {isDefault && (
        <span className="config-chip-badge" title="Printer's default plate">
          printer default
        </span>
      )}
    </div>
  );
}
