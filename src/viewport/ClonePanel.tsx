// Tool panel for cloning objects. Mirrors SplitPanel/PaintPanel/
// ArrangePanel: rendered in the right settings column while a clone
// request is pending (App's .panel-column swaps it in), Esc = cancel.
//
// Two modes, picked by radio:
//   - Copies: exactly N copies, stacked in place on the originals (the
//     user positions them).
//   - Fill plate: clone until the next copy would spill onto another
//     plate, packing with the same nester options as auto-arrange
//     (the shared ArrangeOptionsFields).

import { useEffect, useState } from "react";
import { ArrangeOptionsFields, type ArrangeOptions } from "./ArrangePanel";

/** Parse the copies field into a valid count, or null if not a usable
 *  integer >= 1. Exported for the self-check test. */
export function parseCopies(raw: string): number | null {
  const n = Number(raw);
  if (!Number.isInteger(n) || n < 1) return null;
  return n;
}

export function ClonePanel({
  count,
  arrangeOptions,
  onArrangeOptionsChange,
  onConfirm,
  onClose,
}: {
  /** How many objects are being cloned — for the header label. */
  count: number;
  /** Nester options for fill-plate mode — the same session state the
   *  arrange panel edits. */
  arrangeOptions: ArrangeOptions;
  onArrangeOptionsChange: (next: ArrangeOptions) => void;
  /** `copies` = a number (>= 1) for N copies, or `null` for fill-plate. */
  onConfirm: (copies: number | null) => void;
  onClose: () => void;
}) {
  const [mode, setMode] = useState<"copies" | "fill">("copies");
  const [raw, setRaw] = useState("1");
  const copies = parseCopies(raw);
  const canClone = mode === "fill" || copies !== null;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const confirm = () => {
    if (!canClone) return;
    onConfirm(mode === "fill" ? null : copies);
  };

  return (
    <div className="tool-panel text-neutral-100 text-[13px]">
      <div className="px-3 py-2.5 border-b border-neutral-700 font-medium">
        Clone {count} object{count === 1 ? "" : "s"}
      </div>
      <div className="px-3 py-3 flex flex-col gap-2.5">
        <div className="text-neutral-500">
          Settings and grouping are copied with the geometry.
        </div>
        <label className="flex items-center gap-2.5 cursor-pointer">
          <input
            type="radio"
            name="clone-mode"
            checked={mode === "copies"}
            onChange={() => setMode("copies")}
          />
          <span className="w-14">Copies</span>
          <input
            type="number"
            min={1}
            value={raw}
            disabled={mode !== "copies"}
            autoFocus
            onFocus={() => setMode("copies")}
            onChange={(e) => setRaw(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") confirm();
            }}
            className="w-20 text-right tabular-nums bg-neutral-900 rounded px-2 py-1 disabled:opacity-40"
            aria-label="Number of copies"
          />
        </label>
        <label
          className="flex items-center gap-2.5 cursor-pointer"
          title="Clone until the next copy would need another plate, packing with the arrange options below"
        >
          <input
            type="radio"
            name="clone-mode"
            checked={mode === "fill"}
            onChange={() => setMode("fill")}
          />
          <span>Fill plate</span>
        </label>
      </div>
      {mode === "fill" && (
        <div className="px-3 py-3 flex flex-col gap-2.5 border-t border-neutral-700">
          <div className="text-neutral-500">Arrangement</div>
          <ArrangeOptionsFields
            options={arrangeOptions}
            onChange={onArrangeOptionsChange}
          />
        </div>
      )}
      <div className="px-3 py-3 flex gap-2 justify-end border-t border-neutral-700">
        <button
          type="button"
          className="px-3 py-1.5 rounded hover:bg-neutral-700/60"
          onClick={onClose}
        >
          Cancel
        </button>
        <button
          type="button"
          disabled={!canClone}
          className={`px-3 py-1.5 rounded ${
            canClone
              ? "bg-blue-600 hover:bg-blue-500"
              : "bg-neutral-700 opacity-40 cursor-not-allowed"
          }`}
          onClick={confirm}
        >
          Clone
        </button>
      </div>
    </div>
  );
}
