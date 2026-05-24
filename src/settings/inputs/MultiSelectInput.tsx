// Vector wrapper for the slot-adaptive panel (PR-4-2 + PR-4-6).
//
// libslic3r's vector options carry one entry per slot/extruder
// (`filament_type`, `nozzle_temperature`, `filament_colour`, …).
// PR-4-6's slot-tab-strip renders the **active slot only**; the
// MultiSelectInput is what that single visible row mounts. When
// "Sync edits across slots" is ON the commit broadcasts to every
// index (via `commitVectorEdit`), otherwise it lands at the
// active index only.
//
// PR-4-2 ships the wrapper with the active-index + sync-mode + a
// component-renderer prop. PR-4-6 wires the per-slot rendering on
// top.

import {
  commitVectorEdit,
  formatVector,
  padVector,
  parseVector,
} from "./helpers";
import { defaultVectorFor, type OptionSummary } from "../types";

export interface MultiSelectInputProps {
  schema: OptionSummary;
  /** Raw libslic3r-serialized vector ("0.4,0.4,0.4,0.4") or null. */
  value: string | null;
  onChange: (next: string) => void;
  disabled?: boolean;
  /** Number of slots the active printer declares. The vector is
   *  padded / clipped to this length on commit so libslic3r's
   *  per-extruder enforcement doesn't error. */
  slotCount: number;
  /** Active slot index (1-based — matches the mockup's slot tab
   *  labels, where slot 1 → vector index 0). */
  activeSlot: number;
  /** If true, edits on the active slot broadcast to every index. */
  syncAll: boolean;
  /** Renderer for the active slot's single-value input. The wrapper
   *  threads through value/onChange so the inner control doesn't
   *  need to know it's inside a vector. */
  renderSlot: (props: {
    value: string;
    onChange: (next: string) => void;
    disabled: boolean;
  }) => React.ReactNode;
}

export function MultiSelectInput({
  schema,
  value,
  onChange,
  disabled = false,
  slotCount,
  activeSlot,
  syncAll,
  renderSlot,
}: MultiSelectInputProps) {
  // Use the explicit override when set; otherwise seed from the
  // pre-split default vector the Rust side ships in OptionSummary
  // (typed DefaultValue::Vector). Falls through parseVector either
  // way for the user-edited override-string path.
  const entries = value !== null ? parseVector(value) : defaultVectorFor(schema);
  const padded = padVector(entries, slotCount);
  const idx = Math.max(0, Math.min(activeSlot - 1, slotCount - 1));
  const slotValue = padded[idx] ?? "";
  const raw = value ?? formatVector(padded);

  const commitSlot = (next: string) => {
    const updated = commitVectorEdit(padded, idx, next, syncAll);
    const serialized = formatVector(updated);
    if (serialized !== raw) onChange(serialized);
  };

  return (
    <div className="val-wrap val-vector-wrap" data-active-slot={activeSlot}>
      {renderSlot({ value: slotValue, onChange: commitSlot, disabled })}
    </div>
  );
}
