// Horizontal pill strip of slot chips, prefixed by a sync-button-as-
// row-label. Replaces the old vertical-form Slots section of
// SlotBindingPanel (the Materials section there still uses the
// dropdown form until that gets the same chip treatment).
//
// Components in this file:
//
//   - SlotChipStrip: the public component — `<row-label> + <chips>`.
//   - SyncSlotsLabel: the row label, which doubles as a sync button.
//     State machine: idle → syncing (spinner) → synced (✓, 900ms) →
//     idle. Disabled while syncing.
//   - SlotChip: one pill per slot — swatch + short label + material
//     tag. Click opens the three-pane FilamentPickerModal (see
//     ./FilamentPickerModal.tsx).
//
// Label conventions live in `printer/printerInstance.ts` — chips
// read `deriveSlotShortLabel` for the chip face and the existing
// `flattenSlots`-produced label for the hover tooltip.

import { useEffect, useRef, useState } from "react";
import {
  deriveSlotShortLabel,
  type FlatSlotOption,
  type PrinterInstance,
  type SlotRef,
} from "../printer/printerInstance";
import {
  FilamentPickerModal,
  type FilamentPickerPick,
} from "./FilamentPickerModal";
import type { FilamentSummary } from "./filamentSummary";

export type { FilamentSummary };

const UNASSIGNED_SWATCH = "#9ca3af";

export interface SlotChipStripProps {
  instance: PrinterInstance;
  /** Flat slot options (the same shape SlotBindingPanel already
   *  computes via `flattenSlots`). One per (extruder, slot). */
  slots: FlatSlotOption[];
  /** Filaments the user can choose from. Display via `display_name`,
   *  store via `identity`. */
  filaments: FilamentSummary[];
  /** Apply the modal's pick — both filament identity and slot color
   *  in one call so the caller can route to its existing per-field
   *  writers (setSlotFilament + setSlotColor). */
  onApplyPick: (ref: SlotRef, pick: FilamentPickerPick) => void;
  /** Sync slot loadout from the printer. Returns a Promise so the
   *  button can show its in-flight spinner; resolves regardless of
   *  whether the driver actually round-tripped (the button just
   *  shows it tried). Placeholder until 7c-2 lands the driver
   *  filament-event listener; today this resolves after ~400ms. */
  onSync: () => Promise<void>;
}

export function SlotChipStrip({
  instance,
  slots,
  filaments,
  onApplyPick,
  onSync,
}: SlotChipStripProps): React.JSX.Element {
  const filamentByIdentity = new Map(
    filaments.map((f) => [f.identity, f] as const),
  );
  const totalExtruders = instance.extruders.length;

  return (
    <div className="sp-config-row sp-config-slots">
      <SyncSlotsLabel
        printerName={instance.display_name}
        onSync={onSync}
      />
      {slots.length === 0 ? (
        <span className="sp-config-slots-empty dim">
          no slots — printer has no extruders configured
        </span>
      ) : (
        slots.map((s) => {
          const ext = instance.extruders[s.ref.extruder];
          const shortLabel = ext
            ? deriveSlotShortLabel(
                s.ref.extruder,
                totalExtruders,
                s.ref.slot,
                ext.slots,
              )
            : "";
          const filEntry = s.filament_identity
            ? filamentByIdentity.get(s.filament_identity) ?? null
            : null;
          // Resolve the slug to its human display name when possible
          // (e.g. `generic-pla` → `Generic PLA`). Falls back to the
          // raw slug when the bundled list doesn't carry it — the
          // user still sees *something* identifying.
          const filamentLabel = filEntry
            ? filEntry.display_name
            : s.filament_identity;
          return (
            <SlotChip
              key={`slot-${s.ref.extruder}-${s.ref.slot}`}
              option={s}
              shortLabel={shortLabel}
              materialTag={
                filEntry
                  ? filEntry.base_type
                  : s.filament_identity
                    ? "?"
                    : "—"
              }
              filamentLabel={filamentLabel}
              filaments={filaments}
              onApplyPick={(pick) => onApplyPick(s.ref, pick)}
            />
          );
        })
      )}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────
// SyncSlotsLabel — "Slots" row label that doubles as a sync button.

type SyncState = "idle" | "syncing" | "synced" | "error";

interface SyncSlotsLabelProps {
  printerName: string;
  onSync: () => Promise<void>;
}

function SyncSlotsLabel({
  printerName,
  onSync,
}: SyncSlotsLabelProps): React.JSX.Element {
  const [state, setState] = useState<SyncState>("idle");
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    },
    [],
  );

  const handleClick = (): void => {
    if (state !== "idle") return;
    setState("syncing");
    setErrorMsg(null);
    let failed: string | null = null;
    Promise.resolve(onSync())
      .catch((err: unknown) => {
        // Capture the reason for the error-triangle tooltip.
        failed =
          typeof err === "string"
            ? err
            : err instanceof Error
              ? err.message
              : "sync failed";
      })
      .finally(() => {
        // Settle to the result as soon as the sync resolves — the
        // real driver readout is fast, so no artificial hold. Show
        // the ✓/✗ briefly, then revert to idle.
        if (failed != null) {
          setErrorMsg(failed);
          setState("error");
        } else {
          setState("synced");
        }
        timerRef.current = setTimeout(() => {
          setState("idle");
          setErrorMsg(null);
        }, 900);
      });
  };

  const title =
    state === "syncing"
      ? `Syncing filament loadout from ${printerName}…`
      : state === "synced"
        ? `Filament loadout synced from ${printerName}`
        : state === "error"
          ? `Sync failed: ${errorMsg ?? "unknown error"}`
          : `Physical loadout — what each slot is spooled with right now.\nClick to sync from ${printerName}.`;

  return (
    <button
      className={`config-row-label slots-sync-label state-${state}`}
      onClick={handleClick}
      disabled={state !== "idle"}
      title={title}
      aria-label="Sync filaments from printer"
      aria-busy={state === "syncing"}
    >
      <span className="slots-sync-ico" aria-hidden>
        {state === "synced" ? (
          <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
            <path
              d="M2.5 6.5l2.2 2.2L9.5 3.8"
              stroke="currentColor"
              strokeWidth="1.7"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        ) : state === "error" ? (
          <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
            <path
              d="M6 1.6L11 10.4H1z"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinejoin="round"
              fill="none"
            />
            <path
              d="M6 5v2.4"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
            />
            <circle cx="6" cy="9" r="0.6" fill="currentColor" />
          </svg>
        ) : (
          <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
            <path
              d="M10 5.5A4 4 0 0 0 3 3.2"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
            />
            <path
              d="M10 1.5V3.5H8"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            <path
              d="M2 6.5A4 4 0 0 0 9 8.8"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
            />
            <path
              d="M2 10.5V8.5H4"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        )}
      </span>
      <span className="slots-sync-text">Slots</span>
    </button>
  );
}

// ─────────────────────────────────────────────────────────────────
// SlotChip — one pill per (extruder, slot). Click opens the
// three-pane FilamentPickerModal.

interface SlotChipProps {
  option: FlatSlotOption;
  shortLabel: string;
  materialTag: string;
  /** Resolved filament display name (e.g. "Generic PLA") for the
   *  tooltip. `null` when the slot is empty. */
  filamentLabel: string | null;
  filaments: FilamentSummary[];
  onApplyPick: (pick: FilamentPickerPick) => void;
}

function SlotChip({
  option,
  shortLabel,
  materialTag,
  filamentLabel,
  filaments,
  onApplyPick,
}: SlotChipProps): React.JSX.Element {
  const [open, setOpen] = useState(false);

  const empty = !option.filament_identity;
  const swatch = option.color ?? UNASSIGNED_SWATCH;
  const tooltip = empty
    ? `${option.label} — click to assign filament`
    : `${option.label} — ${filamentLabel ?? option.filament_identity}\nClick to change`;

  return (
    <>
      <button
        className={`slot-pill${empty ? " empty" : ""}`}
        onClick={() => setOpen(true)}
        title={tooltip}
        aria-haspopup="dialog"
        aria-expanded={open}
      >
        <span
          className="slot-pill-swatch"
          style={empty ? undefined : { background: swatch }}
        />
        <span className="slot-pill-label">{shortLabel}</span>
        <span className="slot-pill-material">{materialTag}</span>
      </button>
      {open && (
        <FilamentPickerModal
          slotId={option.label}
          filaments={filaments}
          currentIdentity={option.filament_identity}
          currentColor={option.color}
          onPick={(pick) => {
            setOpen(false);
            onApplyPick(pick);
          }}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
}
