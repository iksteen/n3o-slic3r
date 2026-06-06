// Simple / Advanced / Expert mode filter (PR-4-3) — FR-UI-2.
//
// Horizontal segmented pill control mirroring the editing-context-
// tab idiom (`.sp-tabs` + `.sp-tab`, docs/dev/design/SettingsPanel.jsx
// :582-601) for visual consistency. Persists the selection in
// localStorage so the user's preferred filter survives reloads.

import { useEffect, useState } from "react";
import type { ModeFilter } from "./categories";

const STORAGE_KEY = "n3o.settings.mode";

const VISIBLE_MODES: ReadonlyArray<{ id: ModeFilter; label: string }> = [
  { id: "simple", label: "Simple" },
  { id: "advanced", label: "Advanced" },
  { id: "expert", label: "Expert" },
];

/** Read the persisted mode if it parses to a known variant.
 *  Develop is dev-only and never persists. */
function readStoredMode(): ModeFilter {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw === "simple" || raw === "advanced" || raw === "expert") return raw;
  } catch {
    // localStorage may be disabled — fall through to default.
  }
  return "simple";
}

function writeStoredMode(mode: ModeFilter): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, mode);
  } catch {
    // ignore quota / disabled storage
  }
}

export interface ModeFilterProps {
  value: ModeFilter;
  onChange: (next: ModeFilter) => void;
  /** Show the Develop tab only in dev mode (the Settings panel
   *  passes `import.meta.env.DEV` through). */
  allowDevelop?: boolean;
}

/** Hook variant that owns the persisted state itself. Components
 *  that want full control pass the raw `ModeFilter` component
 *  with their own value/onChange instead. */
export function useStoredModeFilter(): [ModeFilter, (m: ModeFilter) => void] {
  const [mode, setMode] = useState<ModeFilter>(() => readStoredMode());
  useEffect(() => writeStoredMode(mode), [mode]);
  return [mode, setMode];
}

export function ModeFilter({
  value,
  onChange,
  allowDevelop = false,
}: ModeFilterProps) {
  const modes = allowDevelop
    ? [...VISIBLE_MODES, { id: "develop" as const, label: "Develop" }]
    : VISIBLE_MODES;
  return (
    <div className="sp-tabs sp-tabs-mode" role="tablist" aria-label="Setting mode filter">
      {modes.map((m) => (
        <button
          key={m.id}
          type="button"
          role="tab"
          aria-selected={value === m.id}
          className={`sp-tab${value === m.id ? " active" : ""}`}
          onClick={() => onChange(m.id)}
        >
          {m.label}
        </button>
      ))}
    </div>
  );
}
