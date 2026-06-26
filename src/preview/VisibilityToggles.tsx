// Travel + retraction visibility toggles.
//
// Two checkboxes. The renderer's prop interface already accepts
// `showTravels` / `showRetractions`; this component just exposes
// them to the user and persists the user's preference across
// sessions via localStorage.
//
// Defaults: both off. Travels are visual noise on dense prints;
// retractions are debug-only.

import { useCallback, useEffect, useState } from "react";

const LS_TRAVELS = "n3o-slic3r:preview:show-travels";
const LS_RETRACTIONS = "n3o-slic3r:preview:show-retractions";

export interface VisibilityState {
  showTravels: boolean;
  showRetractions: boolean;
}

export interface VisibilityTogglesProps {
  value: VisibilityState;
  onChange: (next: VisibilityState) => void;
}

export function VisibilityToggles({
  value,
  onChange,
}: VisibilityTogglesProps) {
  return (
    <div className="visibility-toggles" role="group" aria-label="Geometry visibility">
      <label className="visibility-toggle">
        <input
          type="checkbox"
          checked={value.showTravels}
          onChange={(e) =>
            onChange({ ...value, showTravels: e.target.checked })
          }
        />
        <span>Travels</span>
      </label>
      <label className="visibility-toggle">
        <input
          type="checkbox"
          checked={value.showRetractions}
          onChange={(e) =>
            onChange({ ...value, showRetractions: e.target.checked })
          }
        />
        <span>Retractions</span>
      </label>
    </div>
  );
}

/** Hook that owns the toggle state + localStorage round-trip.
 * Callers spread the returned `value` into VisibilityToggles +
 * pass `onChange` through. Renderer reads `value.showTravels`
 * and `value.showRetractions` directly. */
export function useVisibilityToggles(): {
  value: VisibilityState;
  onChange: (next: VisibilityState) => void;
} {
  const [value, setValue] = useState<VisibilityState>(() => readStored());

  const onChange = useCallback((next: VisibilityState) => {
    setValue(next);
    writeStored(next);
  }, []);

  // Re-read on mount in case another window updated localStorage
  // between renders — defensive; the hook usually owns it.
  useEffect(() => {
    setValue(readStored());
  }, []);

  return { value, onChange };
}

function readStored(): VisibilityState {
  return {
    showTravels: readBool(LS_TRAVELS, false),
    showRetractions: readBool(LS_RETRACTIONS, false),
  };
}

function writeStored(v: VisibilityState): void {
  writeBool(LS_TRAVELS, v.showTravels);
  writeBool(LS_RETRACTIONS, v.showRetractions);
}

function readBool(key: string, fallback: boolean): boolean {
  try {
    const raw = window.localStorage.getItem(key);
    if (raw === "true") return true;
    if (raw === "false") return false;
    return fallback;
  } catch {
    return fallback;
  }
}

function writeBool(key: string, v: boolean): void {
  try {
    window.localStorage.setItem(key, String(v));
  } catch {
    // ignore: storage disabled / quota
  }
}
