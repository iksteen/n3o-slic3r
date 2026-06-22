import { useEffect, useState } from "react";
import { onEvents } from "../state/eventRouter";
import { pushLog } from "../logging/logStore";
import type { SceneEvent } from "./types";

type Toast = { id: number; text: string };
let nextId = 1;

/**
 * Surfaces the backend's scene-warning events (out-of-bounds, arrange-overflow)
 * as a transient toast + a Console entry. Mounted once, outside the viewport
 * switch, so it covers whichever viewport is active. Extend by adding the event
 * name + a case below.
 */
export function ViewportToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  useEffect(() => {
    const push = (text: string) => {
      pushLog("warn", text); // also surface in the Console drawer, like ViewportCanvas
      const id = nextId++;
      setToasts((t) => [...t, { id, text }]);
      window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 5000);
    };
    return onEvents<SceneEvent>(
      ["scene:object_out_of_bounds", "scene:auto_arrange_overflow"],
      (e) => {
        const ev = e.payload;
        if (ev.kind === "ObjectOutOfBounds") {
          push(
            `object ${ev.data.object_id} out of bounds: ${ev.data.reasons
              .map((r) => r.kind)
              .join(", ")}`,
          );
        } else if (ev.kind === "AutoArrangeOverflow") {
          push(`auto-arrange could not place ${ev.data.un_placed.length} object(s)`);
        }
      },
    );
  }, []);

  return (
    <div
      className="absolute bottom-12 right-3 flex flex-col gap-1 pointer-events-none"
      style={{ zIndex: 20 }}
    >
      {toasts.map((t) => (
        <div
          key={t.id}
          className="pointer-events-auto text-xs px-3 py-2 rounded shadow text-neutral-100 bg-amber-700/90"
        >
          {t.text}
        </div>
      ))}
    </div>
  );
}
