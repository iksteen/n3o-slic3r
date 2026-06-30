import { useEffect, useState } from "react";

// Floating controls for the split (cut-by-plane) tool — rotation sliders for
// the cutting plane, keep/discard per side, and Apply/Cancel. A floating
// overlay (not a modal) so the viewport stays interactive while the plane is
// dragged. Mirrors the ViewportChrome overlay idiom.

type Vec3 = [number, number, number];

const DEG = 180 / Math.PI;
const RAD = Math.PI / 180;

/** Editable degree field: commits a parsed value live, but keeps the raw typed
 *  text while focused so partial entries ("-", "45.") aren't clobbered by the
 *  controlled value. Re-syncs to the canonical value on blur. */
function AngleInput({
  deg,
  onCommit,
}: {
  deg: number;
  onCommit: (deg: number) => void;
}) {
  const [text, setText] = useState<string | null>(null);
  const display = text ?? String(Math.round(deg * 100) / 100);
  return (
    <input
      type="number"
      step={1}
      value={display}
      onChange={(e) => {
        setText(e.target.value);
        const n = parseFloat(e.target.value);
        if (Number.isFinite(n)) onCommit(n);
      }}
      onBlur={() => setText(null)}
      className="w-12 text-right tabular-nums bg-neutral-900 rounded px-1 py-0.5"
      aria-label="angle (degrees)"
    />
  );
}

export function SplitPanel({
  rot,
  keepPos,
  keepNeg,
  onRot,
  onToggleKeep,
  onApply,
  onCancel,
}: {
  rot: Vec3;
  keepPos: boolean;
  keepNeg: boolean;
  onRot: (axis: 0 | 1 | 2, rad: number) => void;
  onToggleKeep: (side: "pos" | "neg") => void;
  onApply: () => void;
  onCancel: () => void;
}) {
  // Esc cancels the tool (matches the placing tools).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  const canApply = keepPos || keepNeg;

  const slider = (axis: 0 | 1 | 2, label: string) => (
    <label className="flex items-center gap-2">
      <span className="w-3 text-neutral-400">{label}</span>
      <input
        type="range"
        min={-180}
        max={180}
        step={1}
        value={Math.round(rot[axis] * DEG)}
        onChange={(e) => onRot(axis, Number(e.target.value) * RAD)}
        className="flex-1"
      />
      <AngleInput deg={rot[axis] * DEG} onCommit={(d) => onRot(axis, d * RAD)} />
    </label>
  );

  const keepRow = (
    side: "pos" | "neg",
    checked: boolean,
    color: string,
    name: string,
  ) => (
    <label className="flex items-center gap-2 cursor-pointer">
      <input type="checkbox" checked={checked} onChange={() => onToggleKeep(side)} />
      <span
        className="inline-block rounded-sm"
        style={{ width: 11, height: 11, background: color }}
      />
      <span>{name}</span>
      <span className="text-neutral-500">{checked ? "keep" : "discard"}</span>
    </label>
  );

  return (
    <div
      className="absolute top-2 right-2 bg-neutral-800/90 text-neutral-100 text-xs rounded shadow pointer-events-auto"
      style={{ zIndex: 10, width: 220 }}
    >
      <div className="px-3 py-2 border-b border-neutral-700 font-medium">
        Split by plane
      </div>
      <div className="px-3 py-2 flex flex-col gap-1.5">
        <div className="text-neutral-500">Plane rotation</div>
        {slider(0, "X")}
        {slider(1, "Y")}
        {slider(2, "Z")}
      </div>
      <div className="px-3 py-2 flex flex-col gap-1.5 border-t border-neutral-700">
        {keepRow("pos", keepPos, "#4073f2", "Blue side")}
        {keepRow("neg", keepNeg, "#d94040", "Red side")}
      </div>
      <div className="px-3 py-2 flex gap-2 justify-end border-t border-neutral-700">
        <button
          type="button"
          className="px-2 py-1 rounded hover:bg-neutral-700/60"
          onClick={onCancel}
        >
          Cancel
        </button>
        <button
          type="button"
          disabled={!canApply}
          className={`px-2.5 py-1 rounded ${
            canApply
              ? "bg-blue-600 hover:bg-blue-500"
              : "bg-neutral-700 opacity-40 cursor-not-allowed"
          }`}
          onClick={onApply}
          title={canApply ? "Cut along the plane" : "Keep at least one side"}
        >
          Split
        </button>
      </div>
    </div>
  );
}
