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
//     tag. Click opens a small filament-pick popover.
//   - FilamentPopover: lightweight per-chip popover with the same
//     filament list the old <select> dropdown had. Placeholder until
//     the richer FilamentPickerModal (docs/design) lands.
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

export interface FilamentSummary {
  identity: string;
  display_name: string;
  base_type: string;
}

const UNASSIGNED_SWATCH = "#9ca3af";

export interface SlotChipStripProps {
  instance: PrinterInstance;
  /** Flat slot options (the same shape SlotBindingPanel already
   *  computes via `flattenSlots`). One per (extruder, slot). */
  slots: FlatSlotOption[];
  /** Filaments the user can choose from. Display via `display_name`,
   *  store via `identity`. */
  filaments: FilamentSummary[];
  /** Pick (or clear, with null) the filament loaded in a slot. */
  onPickFilament: (ref: SlotRef, identity: string | null) => void;
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
  onPickFilament,
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
              onPickFilament={(identity) => onPickFilament(s.ref, identity)}
            />
          );
        })
      )}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────
// SyncSlotsLabel — "Slots" row label that doubles as a sync button.

type SyncState = "idle" | "syncing" | "synced";

interface SyncSlotsLabelProps {
  printerName: string;
  onSync: () => Promise<void>;
}

function SyncSlotsLabel({
  printerName,
  onSync,
}: SyncSlotsLabelProps): React.JSX.Element {
  const [state, setState] = useState<SyncState>("idle");
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
    Promise.resolve(onSync())
      .catch(() => {
        // Swallow — the visual goes to "synced" briefly either way
        // so the user gets confirmation the button fired. Real
        // error surfacing is a 7c-2 concern.
      })
      .finally(() => {
        // Hold the spinner for a beat even if onSync resolves
        // instantly, so the user perceives the action.
        timerRef.current = setTimeout(() => {
          setState("synced");
          timerRef.current = setTimeout(() => setState("idle"), 900);
        }, 650);
      });
  };

  const title =
    state === "syncing"
      ? `Syncing filament loadout from ${printerName}…`
      : state === "synced"
        ? `Filament loadout synced from ${printerName}`
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
// SlotChip — one pill per (extruder, slot).

interface SlotChipProps {
  option: FlatSlotOption;
  shortLabel: string;
  materialTag: string;
  /** Resolved filament display name (e.g. "Generic PLA") for the
   *  tooltip. `null` when the slot is empty. */
  filamentLabel: string | null;
  filaments: FilamentSummary[];
  onPickFilament: (identity: string | null) => void;
}

function SlotChip({
  option,
  shortLabel,
  materialTag,
  filamentLabel,
  filaments,
  onPickFilament,
}: SlotChipProps): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  // Dismiss on outside click.
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent): void => {
      if (!wrapRef.current) return;
      if (!wrapRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  const empty = !option.filament_identity;
  const swatch = option.color ?? UNASSIGNED_SWATCH;
  const tooltip = empty
    ? `${option.label} — click to load filament`
    : `${option.label} — ${filamentLabel ?? option.filament_identity}\nClick to change`;

  return (
    <div className="slot-chip-wrap" ref={wrapRef}>
      <button
        className={`slot-pill${empty ? " empty" : ""}`}
        onClick={() => setOpen((v) => !v)}
        title={tooltip}
        aria-haspopup="menu"
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
        <FilamentPopover
          currentIdentity={option.filament_identity}
          filaments={filaments}
          onPick={(identity) => {
            setOpen(false);
            onPickFilament(identity);
          }}
        />
      )}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────
// FilamentPopover — minimal per-chip picker. Listed by display_name;
// stores identity. Replaces the old `<select>` dropdown for chip-
// flow. The richer FilamentPickerModal from `docs/design/` is a
// separate future port.

interface FilamentPopoverProps {
  currentIdentity: string | null;
  filaments: FilamentSummary[];
  onPick: (identity: string | null) => void;
}

function FilamentPopover({
  currentIdentity,
  filaments,
  onPick,
}: FilamentPopoverProps): React.JSX.Element {
  return (
    <div className="printer-picker-menu slot-chip-popover" role="menu">
      <button
        type="button"
        className={`ptpm-item${currentIdentity == null ? " active" : ""}`}
        onClick={() => onPick(null)}
      >
        <span className="ptpm-name">— empty —</span>
      </button>
      {filaments.map((f) => (
        <button
          key={f.identity}
          type="button"
          className={`ptpm-item${f.identity === currentIdentity ? " active" : ""}`}
          onClick={() => onPick(f.identity)}
          title={`${f.display_name} · ${f.base_type}`}
        >
          <span className="ptpm-name">{f.display_name}</span>
          <span className="ptpm-detail">{f.base_type}</span>
        </button>
      ))}
      {currentIdentity != null &&
        !filaments.some((f) => f.identity === currentIdentity) && (
          // Slot may carry an identity not in the current bundled
          // list (vendor profile renamed / removed). Surface it
          // verbatim so the user can see what's bound + pick a
          // replacement.
          <button
            type="button"
            className="ptpm-item active"
            onClick={() => onPick(currentIdentity)}
          >
            <span className="ptpm-name">{currentIdentity}</span>
            <span className="ptpm-detail">unknown</span>
          </button>
        )}
    </div>
  );
}
