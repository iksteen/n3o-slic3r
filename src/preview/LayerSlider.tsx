// Layer slider for the preview (PR-6-9).
//
// Owns the LayerWindow state's mode picker + the slider thumb(s).
// Renders three layouts:
//   - single  — one thumb, label `Layer N of M`
//   - up-to   — one thumb, label `Layers 1..N of M`
//   - range   — two thumbs, label `Layers A..B of M`
//
// Keyboard (active when the panel has focus and no input is
// focused, per the global hook):
//   ↑ / ↓                step by 1
//   Shift + ↑ / ↓        step by 10
//   Home / End           jump to first / last
//   1 / 2 / 3            switch to single / up-to / range
//
// State persistence is the parent's job — this component is
// pure-controlled. PR-6-15's App mode toggle re-initializes via
// `defaultWindow(layerCount)` on each preview load.

import { useEffect } from "react";

import {
  defaultWindow,
  jumpTo,
  stepLayer,
  switchMode,
} from "./layerWindow";
import type { LayerWindow } from "./types";

export interface LayerSliderProps {
  layerCount: number;
  value: LayerWindow;
  onChange: (next: LayerWindow) => void;
}

export function LayerSlider({
  layerCount,
  value,
  onChange,
}: LayerSliderProps) {
  // Global keyboard shortcuts. Guarded so typing in an input
  // doesn't hijack the slider.
  useEffect(() => {
    if (layerCount <= 0) return;
    const onKey = (e: KeyboardEvent): void => {
      if (isTextInputFocused()) return;
      const step = e.shiftKey ? 10 : 1;
      switch (e.key) {
        case "ArrowUp":
          e.preventDefault();
          onChange(stepLayer(value, step, layerCount));
          return;
        case "ArrowDown":
          e.preventDefault();
          onChange(stepLayer(value, -step, layerCount));
          return;
        case "Home":
          e.preventDefault();
          onChange(jumpTo(value, "first", layerCount));
          return;
        case "End":
          e.preventDefault();
          onChange(jumpTo(value, "last", layerCount));
          return;
        case "1":
          if (value.mode !== "single") {
            e.preventDefault();
            onChange(switchMode(value, "single"));
          }
          return;
        case "2":
          if (value.mode !== "up-to") {
            e.preventDefault();
            onChange(switchMode(value, "up-to"));
          }
          return;
        case "3":
          if (value.mode !== "range") {
            e.preventDefault();
            onChange(switchMode(value, "range"));
          }
          return;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [value, layerCount, onChange]);

  if (layerCount <= 0) {
    return (
      <div className="layer-slider layer-slider-empty" aria-disabled="true">
        <span className="layer-slider-label">No layers</span>
      </div>
    );
  }

  const last = layerCount - 1;

  return (
    <div className="layer-slider">
      <ModePicker
        current={value.mode}
        onChange={(mode) => onChange(switchMode(value, mode))}
      />
      <SliderBody
        value={value}
        last={last}
        onChange={onChange}
      />
      <span className="layer-slider-label">{labelFor(value, layerCount)}</span>
    </div>
  );
}

function ModePicker({
  current,
  onChange,
}: {
  current: LayerWindow["mode"];
  onChange: (mode: LayerWindow["mode"]) => void;
}) {
  return (
    <div className="layer-slider-modes" role="radiogroup" aria-label="Layer view mode">
      <ModeButton mode="single" current={current} onChange={onChange} label="Single" />
      <ModeButton mode="up-to" current={current} onChange={onChange} label="Up to" />
      <ModeButton mode="range" current={current} onChange={onChange} label="Range" />
    </div>
  );
}

function ModeButton({
  mode,
  current,
  onChange,
  label,
}: {
  mode: LayerWindow["mode"];
  current: LayerWindow["mode"];
  onChange: (mode: LayerWindow["mode"]) => void;
  label: string;
}) {
  const active = current === mode;
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      className={`layer-slider-mode${active ? " active" : ""}`}
      onClick={() => onChange(mode)}
      title={`${label} layer view`}
    >
      {label}
    </button>
  );
}

function SliderBody({
  value,
  last,
  onChange,
}: {
  value: LayerWindow;
  last: number;
  onChange: (next: LayerWindow) => void;
}) {
  if (value.mode === "range") {
    // Two thumbs, range-mode. Each thumb is clamped so it can't
    // cross the other.
    return (
      <div className="layer-slider-track">
        <input
          type="range"
          min={0}
          max={last}
          value={value.min}
          onChange={(e) => {
            const next = Math.min(Number(e.target.value), value.max);
            onChange({ mode: "range", min: next, max: value.max });
          }}
          aria-label="Range min"
        />
        <input
          type="range"
          min={0}
          max={last}
          value={value.max}
          onChange={(e) => {
            const next = Math.max(Number(e.target.value), value.min);
            onChange({ mode: "range", min: value.min, max: next });
          }}
          aria-label="Range max"
        />
      </div>
    );
  }

  const current = value.mode === "single" ? value.layer : value.max;
  return (
    <div className="layer-slider-track">
      <input
        type="range"
        min={0}
        max={last}
        value={current}
        onChange={(e) => {
          const n = Number(e.target.value);
          if (value.mode === "single") {
            onChange({ mode: "single", layer: n });
          } else {
            onChange({ mode: "up-to", max: n });
          }
        }}
        aria-label={value.mode === "single" ? "Layer" : "Up to layer"}
      />
    </div>
  );
}

function labelFor(value: LayerWindow, layerCount: number): string {
  switch (value.mode) {
    case "single":
      return `Layer ${value.layer + 1} of ${layerCount}`;
    case "up-to":
      return `Layers 1..${value.max + 1} of ${layerCount}`;
    case "range":
      return `Layers ${value.min + 1}..${value.max + 1} of ${layerCount}`;
  }
}

function isTextInputFocused(): boolean {
  const el = document.activeElement;
  if (!el) return false;
  const tag = el.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") {
    return true;
  }
  return (el as HTMLElement).isContentEditable === true;
}

// Re-export defaultWindow so panel consumers don't have to
// reach into layerWindow.ts.
export { defaultWindow };
