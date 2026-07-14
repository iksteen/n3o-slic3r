// Tool panel for auto-arrange. Mirrors SplitPanel/PaintPanel: rendered in
// the right settings column while the tool is open (App's .panel-column
// swaps it in), Esc = cancel. The options are pure UI state (App holds
// them so they survive panel close/reopen within the session) and are
// passed straight to `scene_auto_arrange` — nothing is persisted
// backend-side.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface ArrangeOptions {
  spacing_mm: number;
  allow_rotations: boolean;
}

/** The nester option rows (spacing + allow-rotation). Shared between
 *  the arrange panel and the clone panel's fill-plate mode, which packs
 *  with the same options. */
export function ArrangeOptionsFields({
  options,
  onChange,
}: {
  options: ArrangeOptions;
  onChange: (next: ArrangeOptions) => void;
}) {
  // Keep raw text while the spacing field is focused so a partial entry
  // like "2." isn't clobbered by the parsed round-trip (same pattern as
  // SplitPanel's AngleInput).
  const [spacingText, setSpacingText] = useState<string | null>(null);
  return (
    <>
      <label
        className="flex items-center gap-2.5"
        title="Minimum gap between objects placed by auto-arrange"
      >
        <span className="text-neutral-500 w-16">Spacing</span>
        <input
          type="number"
          min={0}
          max={100}
          step={0.5}
          value={spacingText ?? String(options.spacing_mm)}
          onChange={(e) => {
            setSpacingText(e.target.value);
            const n = parseFloat(e.target.value);
            if (Number.isFinite(n)) {
              onChange({ ...options, spacing_mm: Math.min(Math.max(n, 0), 100) });
            }
          }}
          onBlur={() => setSpacingText(null)}
          className="w-20 text-right tabular-nums bg-neutral-900 rounded px-2 py-1"
          aria-label="Arrange spacing (millimeters)"
        />
        <span className="text-neutral-500">mm</span>
      </label>
      <label
        className="flex items-center gap-2.5 cursor-pointer py-0.5"
        title="Let auto-arrange rotate objects for a tighter pack (replaces their authored orientation)"
      >
        <input
          type="checkbox"
          checked={options.allow_rotations}
          onChange={(e) =>
            onChange({ ...options, allow_rotations: e.target.checked })
          }
        />
        <span>Allow rotation</span>
      </label>
    </>
  );
}

export function ArrangePanel({
  options,
  onChange,
  onClose,
}: {
  options: ArrangeOptions;
  onChange: (next: ArrangeOptions) => void;
  onClose: () => void;
}) {
  const [arranging, setArranging] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !arranging) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, arranging]);

  const runArrange = () => {
    if (arranging) return;
    setArranging(true);
    void invoke("scene_auto_arrange", {
      spacingMm: options.spacing_mm,
      allowRotations: options.allow_rotations,
    })
      .then(() => onClose())
      .catch((e) => console.error("arrange failed", e))
      .finally(() => setArranging(false));
  };

  return (
    <div className="tool-panel text-neutral-100 text-[13px]">
      <div className="px-3 py-2.5 border-b border-neutral-700 font-medium">
        Arrange
      </div>
      <div className="px-3 py-3 flex flex-col gap-2.5">
        <ArrangeOptionsFields options={options} onChange={onChange} />
      </div>
      <div className="px-3 py-3 flex gap-2 justify-end border-t border-neutral-700">
        <button
          type="button"
          disabled={arranging}
          className="px-3 py-1.5 rounded hover:bg-neutral-700/60 disabled:opacity-40 disabled:cursor-not-allowed"
          onClick={onClose}
        >
          Cancel
        </button>
        <button
          type="button"
          disabled={arranging}
          className={`px-3 py-1.5 rounded ${
            arranging
              ? "bg-neutral-700 opacity-40 cursor-not-allowed"
              : "bg-blue-600 hover:bg-blue-500"
          }`}
          onClick={runArrange}
          title="Pack the plate with these options"
        >
          {arranging ? "Arranging…" : "Arrange"}
        </button>
      </div>
    </div>
  );
}
