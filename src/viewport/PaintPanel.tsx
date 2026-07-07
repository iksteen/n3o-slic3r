import type { ReactNode } from "react";
import { useEffect } from "react";

// Floating panel for the support-paint tool (manual tree/normal supports).
// Mirrors SplitPanel: a small overlay top-right, Esc = cancel. Brush controls
// (radius, shape, smart-fill + angle) live here; the enforce/block/erase action
// is chosen by mouse button (LMB enforce, RMB block, Shift = erase) in the
// viewport, so it's shown here only as a legend.

const isAuto = (t: string | null) => t === "tree(auto)" || t === "normal(auto)";

// One "the paint won't take effect — enable this support type" prompt, with the
// tree/normal one-click fixes.
function EnableHint({
  text,
  onTree,
  onNormal,
}: {
  text: ReactNode;
  onTree: () => void;
  onNormal: () => void;
}) {
  return (
    <div className="px-3 py-2 border-t border-neutral-700 flex flex-col gap-1.5">
      <span className="text-neutral-500 text-[11px]">{text}</span>
      <div className="flex gap-1 [&>button]:flex-1">
        <button
          type="button"
          className="px-2 py-1 rounded bg-neutral-700 hover:bg-neutral-600"
          onClick={onTree}
        >
          Tree
        </button>
        <button
          type="button"
          className="px-2 py-1 rounded bg-neutral-700 hover:bg-neutral-600"
          onClick={onNormal}
        >
          Normal
        </button>
      </div>
    </div>
  );
}

export function PaintPanel({
  radius,
  onRadius,
  brush,
  onBrush,
  fill,
  onFill,
  angle,
  onAngle,
  supportType,
  enableSupport,
  hasEnforce,
  hasBlock,
  onEnableSupport,
  onErase,
  onClose,
}: {
  radius: number;
  onRadius: (r: number) => void;
  brush: number; // 0 circle, 1 sphere
  onBrush: (b: number) => void;
  fill: boolean;
  onFill: (f: boolean) => void;
  angle: number;
  onAngle: (a: number) => void;
  /** The plate's current `support_type` override, or null when inheriting the
   *  cascade default. */
  supportType: string | null;
  /** Whether `enable_support` is on for the plate. */
  enableSupport: boolean;
  /** Whether the object currently carries enforcer / blocker paint. */
  hasEnforce: boolean;
  hasBlock: boolean;
  /** One-click set the plate to `enable_support` + the given `support_type` so
   *  the painted enforcers/blockers actually take effect. */
  onEnableSupport: (
    type: "tree(manual)" | "normal(manual)" | "tree(auto)" | "normal(auto)",
  ) => void;
  /** Erase all paint on the object (tool stays open). */
  onErase: () => void;
  /** Close the tool (paint is already applied live). */
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const toggle = (on: boolean, label: string, click: () => void, title?: string) => (
    <button
      type="button"
      className={`px-2 py-0.5 rounded ${
        on ? "bg-blue-600" : "bg-neutral-700 hover:bg-neutral-600"
      }`}
      onClick={click}
      title={title}
      aria-pressed={on}
    >
      {label}
    </button>
  );

  return (
    <div
      className="absolute top-12 right-2 bg-neutral-800/90 text-neutral-100 text-xs rounded shadow pointer-events-auto"
      style={{ zIndex: 10, width: 220 }}
    >
      <div className="px-3 py-2 border-b border-neutral-700 font-medium">
        Paint supports
      </div>

      <div className="px-3 py-2 flex flex-col gap-2">
        <div className="flex gap-1 [&>button]:flex-1">
          {toggle(!fill && brush === 0, "Circle", () => { onBrush(0); onFill(false); }, "Paint front faces only")}
          {toggle(!fill && brush === 1, "Sphere", () => { onBrush(1); onFill(false); }, "Paint through the model")}
          {toggle(fill, "Fill", () => onFill(true), "Smart fill: click floods a flat region")}
        </div>
        {fill ? (
          <label className="flex items-center gap-2">
            <span className="text-neutral-500 w-12">Angle</span>
            <input
              type="range"
              min={0}
              max={90}
              step={1}
              value={angle}
              onChange={(e) => onAngle(Number(e.target.value))}
              className="flex-1 m-0 min-w-0"
            />
            <span className="text-right tabular-nums">{angle}°</span>
          </label>
        ) : (
          <>
            <label className="flex items-center gap-2">
              <span className="text-neutral-500 w-12">Radius</span>
              <input
                type="range"
                min={0.2}
                max={8}
                step={0.1}
                value={radius}
                onChange={(e) => onRadius(Number(e.target.value))}
                className="flex-1 m-0 min-w-0"
              />
              <span className="text-right tabular-nums">{radius.toFixed(1)}</span>
            </label>
            <div className="text-neutral-500 text-[11px]">Ctrl+Wheel · Resize the brush</div>
          </>
        )}
      </div>

      <div className="px-3 py-2 flex flex-col gap-1 border-t border-neutral-700 text-[11px] text-neutral-400">
        <div>
          <span className="inline-block rounded-full align-middle" style={{ width: 10, height: 10, background: "#22c55e" }} /> LMB · Enforce
        </div>
        <div>
          <span className="inline-block rounded-full align-middle" style={{ width: 10, height: 10, background: "#e64040" }} /> RMB · Block
        </div>
        <div>
          <span className="inline-block rounded-full align-middle" style={{ width: 10, height: 10, background: "#bfbfc2" }} /> Shift+LMB/RMB · Erase
        </div>
        <div>Ctrl+Z · Undo stroke</div>
      </div>

      {hasEnforce && !enableSupport && (
        <EnableHint
          text={
            <>
              Painted enforcers only grow supports when <b>support is enabled</b>.
            </>
          }
          onTree={() => onEnableSupport("tree(manual)")}
          onNormal={() => onEnableSupport("normal(manual)")}
        />
      )}

      {hasBlock && !(enableSupport && isAuto(supportType)) && (
        <EnableHint
          text={
            <>
              Painted blockers only remove supports under an <b>auto</b> support type.
            </>
          }
          onTree={() => onEnableSupport("tree(auto)")}
          onNormal={() => onEnableSupport("normal(auto)")}
        />
      )}

      <div className="px-3 py-2 flex gap-2 justify-between border-t border-neutral-700">
        <button
          type="button"
          className="px-2 py-1 rounded bg-neutral-700 hover:bg-neutral-600"
          onClick={onErase}
          title="Remove all painted supports from this object"
        >
          Erase all
        </button>
        <button
          type="button"
          className="px-2.5 py-1 rounded bg-blue-600 hover:bg-blue-500"
          onClick={onClose}
          title="Close the paint tool (painted supports stay applied)"
        >
          Close
        </button>
      </div>
    </div>
  );
}
