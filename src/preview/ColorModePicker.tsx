// Color-mode picker for the preview.
//
// Radio group over the 5 color modes + a palette checkbox
// (checked = slicer-classic, unchecked = CVD-safe default).
// State is owned via useColorModePicker() with localStorage
// persistence — survives reloads, restores the last-active mode
// on the next preview.

import { useCallback, useEffect, useState } from "react";

import type { ColorMode, Palette } from "./types";

const LS_MODE = "n3o-slic3r:preview:color-mode";
const LS_PALETTE = "n3o-slic3r:preview:palette";

const MODES: { mode: ColorMode; label: string }[] = [
  { mode: "Feature", label: "Feature" },
  { mode: "Speed", label: "Speed" },
  { mode: "Flow", label: "Flow" },
  { mode: "LayerTime", label: "Layer time" },
  { mode: "Tool", label: "Tool" },
];

export interface ColorModePickerProps {
  mode: ColorMode;
  palette: Palette;
  onChange: (next: { mode: ColorMode; palette: Palette }) => void;
}

export function ColorModePicker({
  mode,
  palette,
  onChange,
}: ColorModePickerProps) {
  return (
    <div className="color-mode-picker">
      <div
        className="color-mode-options"
        role="radiogroup"
        aria-label="Color mode"
      >
        {MODES.map((m) => (
          <label key={m.mode} className="color-mode-option">
            <input
              type="radio"
              name="preview-color-mode"
              checked={mode === m.mode}
              onChange={() => onChange({ mode: m.mode, palette })}
            />
            <span>{m.label}</span>
          </label>
        ))}
      </div>
      <label
        className="palette-toggle"
        title="Use the slicer-classic palette instead of the CVD-safe Okabe-Ito + viridis default"
      >
        <input
          type="checkbox"
          checked={palette === "Classic"}
          onChange={(e) =>
            onChange({ mode, palette: e.target.checked ? "Classic" : "Default" })
          }
        />
        <span>Classic palette</span>
      </label>
    </div>
  );
}

/** State hook owning the picker's persistence. The caller
 * spreads `state` into the picker + reads `state.mode` /
 * `state.palette` to drive the renderer. */
export function useColorModePicker(): {
  state: { mode: ColorMode; palette: Palette };
  onChange: (next: { mode: ColorMode; palette: Palette }) => void;
} {
  const [state, setState] = useState<{
    mode: ColorMode;
    palette: Palette;
  }>(() => readStored());
  const onChange = useCallback(
    (next: { mode: ColorMode; palette: Palette }) => {
      setState(next);
      writeStored(next);
    },
    [],
  );
  useEffect(() => {
    setState(readStored());
  }, []);
  return { state, onChange };
}

function readStored(): { mode: ColorMode; palette: Palette } {
  let mode: ColorMode = "Feature";
  let palette: Palette = "Default";
  try {
    const m = window.localStorage.getItem(LS_MODE);
    if (m && MODES.some((x) => x.mode === m)) {
      mode = m as ColorMode;
    }
    const p = window.localStorage.getItem(LS_PALETTE);
    if (p === "Default" || p === "Classic") {
      palette = p;
    }
  } catch {
    // ignore: storage disabled
  }
  return { mode, palette };
}

function writeStored(v: { mode: ColorMode; palette: Palette }): void {
  try {
    window.localStorage.setItem(LS_MODE, v.mode);
    window.localStorage.setItem(LS_PALETTE, v.palette);
  } catch {
    // ignore
  }
}
