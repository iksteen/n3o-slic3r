// Color-mode picker for the preview (PR-6-13).
//
// Segmented control over the 5 color modes + a small palette
// toggle. State is owned via useColorModePicker() with
// localStorage persistence — survives reloads, restores the
// last-active mode on the next preview.

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
        className="color-mode-buttons"
        role="radiogroup"
        aria-label="Color mode"
      >
        {MODES.map((m) => (
          <button
            key={m.mode}
            type="button"
            role="radio"
            aria-checked={mode === m.mode}
            className={`color-mode-button${mode === m.mode ? " active" : ""}`}
            onClick={() => onChange({ mode: m.mode, palette })}
          >
            {m.label}
          </button>
        ))}
      </div>
      <button
        type="button"
        className="palette-toggle"
        onClick={() =>
          onChange({
            mode,
            palette: palette === "Default" ? "Classic" : "Default",
          })
        }
        title={
          palette === "Default"
            ? "CVD-safe Okabe-Ito + viridis palette (toggle to slicer-classic)"
            : "Slicer-classic palette (toggle to CVD-safe default)"
        }
      >
        Palette: {palette === "Default" ? "CVD-safe" : "Classic"}
      </button>
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
