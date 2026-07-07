import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

// Owns the support-paint tool session (manual tree/normal supports). Like
// `useSplitSession`, this is a modal editing session over one object: `enter`
// opens the Rust-side paint session (which seeds from the object's existing
// support paint and drives a live libslic3r TriangleSelector), the brush
// settings live here, and `apply`/`exit` close it. The actual strokes are
// invoked from `WgpuViewport`'s pointer handlers (they need the live camera).

// Defaults: circle brush, radius 1.0mm, smart-fill angle 30°.
const DEFAULT_RADIUS = 1.0;
const RADIUS_MIN = 0.2;
const RADIUS_MAX = 8.0;
const DEFAULT_ANGLE = 30;

/** enforcer/blocker presence, returned by the paint commands. */
export type PaintFlags = { enforce: boolean; block: boolean };

export interface PaintSession {
  active: boolean;
  objectId: number | null;
  /** Brush radius, world mm. Ctrl+wheel + the panel slider drive it. */
  radius: number;
  setRadius: (r: number) => void;
  /** 0 = circle (screen-projected), 1 = sphere. */
  brush: number;
  setBrush: (b: number) => void;
  /** Smart-fill angle bound, degrees. */
  angle: number;
  setAngle: (a: number) => void;
  /** Smart-fill mode: a click floods the angle-bounded region instead of
   *  brushing. */
  fill: boolean;
  setFill: (f: boolean) => void;
  /** Open the tool on `objectId` (seeds from its existing support paint). */
  enter: (objectId: number) => void;
  /** Close the tool. Paint is applied live per stroke, so this just finalizes. */
  exit: () => void;
  /** Erase all paint on the object (keeps the tool open). */
  clear: () => void;
  /** Bumped after an out-of-band overlay change (Erase all) so the viewport,
   *  which renders strokes itself, knows to redraw. */
  epoch: number;
  /** Whether the object currently carries enforcer / blocker paint. Drives the
   *  "enable manual/auto support" prompts. */
  hasEnforce: boolean;
  hasBlock: boolean;
  /** Record the paint flags returned by a stroke/undo (invoked from the
   *  viewport, which owns the stroke commands). */
  notePainted: (enforce: boolean, block: boolean) => void;
}

export function usePaintSession(): PaintSession {
  const [active, setActive] = useState(false);
  const [objectId, setObjectId] = useState<number | null>(null);
  const [radius, setRadius] = useState(DEFAULT_RADIUS);
  const [brush, setBrush] = useState(0); // circle
  const [angle, setAngle] = useState(DEFAULT_ANGLE);
  const [fill, setFill] = useState(false);
  const [epoch, setEpoch] = useState(0);
  const [hasEnforce, setHasEnforce] = useState(false);
  const [hasBlock, setHasBlock] = useState(false);

  const notePainted = (enforce: boolean, block: boolean): void => {
    setHasEnforce(enforce);
    setHasBlock(block);
  };

  const clampRadius = (r: number): number =>
    Math.min(RADIUS_MAX, Math.max(RADIUS_MIN, r));

  const enter = (id: number): void => {
    setObjectId(id);
    setActive(true);
    setFill(false);
    void invoke<PaintFlags>("paint_open", { objectId: id })
      .then((f) => notePainted(f.enforce, f.block))
      .catch((e: unknown) => {
        console.error("[paint] open failed", e);
        setActive(false);
        setObjectId(null);
      });
  };

  const exit = (): void => {
    setActive(false);
    setObjectId(null);
    void invoke("paint_close").catch((e: unknown) =>
      console.error("[paint] close failed", e),
    );
  };

  const clear = (): void => {
    void invoke<PaintFlags>("paint_clear")
      .then((f) => {
        notePainted(f.enforce, f.block);
        setEpoch((n) => n + 1);
      })
      .catch((e: unknown) => console.error("[paint] clear failed", e));
  };

  return {
    active,
    objectId,
    radius,
    setRadius: (r) => setRadius(clampRadius(r)),
    brush,
    setBrush,
    angle,
    setAngle,
    fill,
    setFill,
    enter,
    exit,
    clear,
    epoch,
    hasEnforce,
    hasBlock,
    notePainted,
  };
}
