// Per-slot tab strip + sync-edit toggle (PR-4-6) — FR-UI-8.
//
// When the active printer has `slot_count >= 2`, the panel renders
// this strip above the settings list. Vector-typed options
// (filament_type, nozzle_temperature, …) show the active slot's
// value only; commits land at that slot's vector index. With the
// sync toggle ON (default), commits broadcast to every slot — the
// common "configure all toolheads identically" case.
//
// `slot_count == 1` printers don't render the strip at all (the
// host elides the component).

import { useEffect, useState } from "react";

const SYNC_STORAGE_KEY = "n3o.settings.sync_slots";

export interface SlotInfo {
  /** 1-based slot index (slot 1 → vector index 0). */
  index: number;
  /** Optional filament color for the swatch dot. */
  color?: string | null;
  /** Optional label override; defaults to the slot index. */
  label?: string;
}

export interface SlotTabStripProps {
  slots: readonly SlotInfo[];
  activeSlot: number;
  onActiveSlotChange: (next: number) => void;
  syncAll: boolean;
  onSyncAllChange: (next: boolean) => void;
}

/** Hook that owns the active-slot + syncAll state. SyncAll persists
 *  to localStorage so the user's preference survives reloads;
 *  active-slot is session-only since it depends on selection state. */
export function useSlotState(
  initialSlotCount: number,
): {
  activeSlot: number;
  setActiveSlot: (n: number) => void;
  syncAll: boolean;
  setSyncAll: (b: boolean) => void;
} {
  const [activeSlot, setActiveSlot] = useState<number>(1);
  const [syncAll, setSyncAll] = useState<boolean>(() => readStoredSync());
  useEffect(() => writeStoredSync(syncAll), [syncAll]);
  // If the slot count drops below the current active slot (e.g.
  // printer change from U1 → A1 mini), clamp.
  useEffect(() => {
    if (activeSlot > initialSlotCount) setActiveSlot(1);
  }, [initialSlotCount, activeSlot]);
  return { activeSlot, setActiveSlot, syncAll, setSyncAll };
}

function readStoredSync(): boolean {
  try {
    const raw = window.localStorage.getItem(SYNC_STORAGE_KEY);
    return raw === null ? true : raw === "true";
  } catch {
    return true;
  }
}

function writeStoredSync(v: boolean): void {
  try {
    window.localStorage.setItem(SYNC_STORAGE_KEY, String(v));
  } catch {
    // ignore quota / disabled
  }
}

export function SlotTabStrip({
  slots,
  activeSlot,
  onActiveSlotChange,
  syncAll,
  onSyncAllChange,
}: SlotTabStripProps) {
  if (slots.length < 2) return null;
  return (
    <div className="slot-tab-strip" role="tablist" aria-label="Filament slots">
      <div className="slot-tabs">
        {slots.map((s) => {
          const active = s.index === activeSlot;
          return (
            <button
              key={s.index}
              type="button"
              role="tab"
              aria-selected={active}
              className={`slot-tab${active ? " active" : ""}`}
              onClick={() => onActiveSlotChange(s.index)}
            >
              {s.color && (
                <span
                  className="slot-tab-swatch"
                  style={{ background: s.color }}
                  aria-hidden
                />
              )}
              <span className="slot-tab-label">{s.label ?? s.index}</span>
            </button>
          );
        })}
      </div>
      <label className="slot-sync-toggle">
        <input
          type="checkbox"
          checked={syncAll}
          onChange={(e) => onSyncAllChange(e.target.checked)}
        />
        <span>Sync edits across slots</span>
      </label>
    </div>
  );
}
