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
  type FlatSlotOption,
  type PrinterInstance,
  type SlotRef,
} from "../printer/printerInstance";
import {
  FilamentPickerModal,
  type FilamentPickerPick,
} from "./FilamentPickerModal";
import { FilamentSettingsModal } from "./FilamentSettingsModal";
import { CloneFilamentDialog } from "./CloneFilamentDialog";
import {
  revertUserFilament,
  deleteUserFilament,
  cloneUserFilament,
} from "./userFilament";
import { isRfidDetected } from "./materials";
import type { FilamentSummary } from "./filamentSummary";

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
  /** Calibrate pressure advance for a slot's filament on the live
   *  printer and store the measured K. Omitted when no driver is
   *  connected (the chip hides its Calibrate button). */
  onCalibrate?: (ref: SlotRef) => void;
  /** `extruder-slot` key of the slot currently calibrating, so its
   *  chip can show the in-flight state. */
  calibratingKey?: string | null;
}

export function SlotChipStrip({
  instance,
  slots,
  filaments,
  onApplyPick,
  onSync,
  onCalibrate,
  calibratingKey,
}: SlotChipStripProps): React.JSX.Element {
  const filamentByIdentity = new Map(
    filaments.map((f) => [f.identity, f] as const),
  );
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
          const shortLabel = s.short_label;
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
              onCalibrate={onCalibrate ? () => onCalibrate(s.ref) : undefined}
              calibrating={
                calibratingKey === `${s.ref.extruder}-${s.ref.slot}`
              }
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
  /** Calibrate PA for this slot on the live printer. Omitted when no
   *  driver is connected. */
  onCalibrate?: () => void;
  /** This slot's PA calibration is in flight. */
  calibrating?: boolean;
}

function SlotChip({
  option,
  shortLabel,
  materialTag,
  filamentLabel,
  filaments,
  onApplyPick,
  onCalibrate,
  calibrating,
}: SlotChipProps): React.JSX.Element {
  const [open, setOpen] = useState(false);
  // Which filament's settings editor is open (over the picker), by slug.
  const [editing, setEditing] = useState<string | null>(null);
  // Which filament is being copied (the clone dialog's source), if any.
  const [copying, setCopying] = useState<FilamentSummary | null>(null);

  const empty = !option.filament_identity;
  const swatch = option.color ?? UNASSIGNED_SWATCH;
  // RFID-detected slots are printer-authoritative (a write would be
  // stomped on the next read) — render read-only, no picker.
  const rfid = isRfidDetected(option.tag_uid);
  const tooltip = rfid
    ? `${option.label} — ${filamentLabel ?? option.filament_identity}\nAuto-detected via RFID — identity managed by the printer.\nClick to view and edit filament settings.`
    : empty
      ? `${option.label} — click to assign filament`
      : `${option.label} — ${filamentLabel ?? option.filament_identity}\nClick to change`;

  return (
    <>
      <button
        className={`slot-pill${empty ? " empty" : ""}${rfid ? " rfid" : ""}`}
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
        {rfid && (
          <span
            className="slot-pill-rfid"
            role="img"
            aria-label="Auto-detected via RFID"
          >
            {/* RFID beacon symbol lifted from the bottom-center icon of
                vecteezy_free-rfid-vector-icon_103817.svg (symbol only,
                wordmark dropped). viewBox cropped to the glyph. */}
            <svg
              width="12"
              height="12"
              viewBox="555 513 290 290"
              fill="currentColor"
              aria-hidden
            >
              <path d="M658.1,592.2c-34.4,37.4-34.4,94.9,0,132.3c5.4-6.2,10.8-12.4,16.3-18.5c-23.9-30.4-23.9-64.8,0-95.3C668.9,604.6,663.5,598.5,658.1,592.2z" />
              <path d="M633.9,574.6c-5.5-7.2-10.8-14.6-15.9-22.2c-57,56.5-57,155.4,0,211.9c5.1-7.5,10.4-14.9,15.9-22.2C589.8,696.5,589.8,620.2,633.9,574.6z" />
              <path d="M700.6,641.1c-0.1,0-0.2,0-0.3,0c-0.1,0-0.2,0-0.3,0c-9.5-0.1-17.3,7.9-17.3,17.3c0,9.4,7.8,17.4,17.3,17.3c0.1,0,0.2,0,0.3,0c0.1,0,0.2,0,0.3,0c9.5,0.1,17.3-7.9,17.3-17.3C717.9,648.9,710.1,641,700.6,641.1z" />
              <path d="M742.4,592.2c-5.4,6.2-10.8,12.4-16.3,18.5c23.9,30.4,23.9,64.8,0,95.3c5.5,6.1,11,12.3,16.3,18.5C776.8,687.1,776.8,629.6,742.4,592.2z" />
              <path d="M782.5,552.4c-5.1,7.5-10.4,14.9-15.9,22.2c44.1,45.6,44.1,121.9,0,167.5c5.5,7.2,10.8,14.6,15.9,22.2C839.5,707.8,839.5,608.9,782.5,552.4z" />
            </svg>
          </span>
        )}
      </button>
      {onCalibrate && !empty && (
        <button
          className="slot-pill-calibrate"
          onClick={onCalibrate}
          disabled={calibrating}
          title={
            "Calibrate pressure advance for this filament on the printer.\n" +
            "Runs FLOW_CALIBRATE on the active toolhead (load this slot's " +
            "toolhead first) and stores the measured K for future slices."
          }
        >
          {calibrating ? "…" : "PA"}
        </button>
      )}
      {open && (
        <FilamentPickerModal
          slotId={option.label}
          filaments={filaments}
          currentIdentity={option.filament_identity}
          currentColor={option.color}
          locked={rfid}
          onPick={(pick) => {
            setOpen(false);
            onApplyPick(pick);
          }}
          onClose={() => setOpen(false)}
          onEdit={(identity) => setEditing(identity)}
          onCopy={(identity) => {
            const src = filaments.find((f) => f.identity === identity);
            if (src) setCopying(src);
          }}
          onRevert={(identity) => {
            if (
              !window.confirm("Revert this filament to its bundled defaults?")
            ) {
              return;
            }
            revertUserFilament(identity).catch((e) =>
              console.error("[filament] revert failed", e),
            );
          }}
          onDelete={(identity) => {
            if (!window.confirm("Delete this custom filament?")) return;
            deleteUserFilament(identity).catch((e) =>
              console.error("[filament] delete failed", e),
            );
          }}
        />
      )}
      {copying && (
        <CloneFilamentDialog
          source={copying}
          materials={Array.from(new Set(filaments.map((f) => f.base_type)))}
          onClose={() => setCopying(null)}
          onClone={(vendor, filamentType) => {
            cloneUserFilament(copying.identity, vendor, filamentType)
              .then((created) => {
                setCopying(null);
                // Show the new clone's settings editor (the request: after
                // saving, open the cloned filament's editor).
                setEditing(created.identity);
              })
              .catch((e) => console.error("[filament] clone failed", e));
          }}
        />
      )}
      {editing && (
        <FilamentSettingsModal
          base={editing}
          name={
            filaments.find((f) => f.identity === editing)?.display_name ??
            editing
          }
          onClose={() => setEditing(null)}
        />
      )}
    </>
  );
}
