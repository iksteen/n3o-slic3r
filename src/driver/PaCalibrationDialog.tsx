// Dialog for the manual PA-Line calibration test print. Pick a loaded slot and
// a K sweep (start/end/step), then slice + send a test print to the printer.
// The user reads the best line off the print by eye and types that K into the
// Flow Dynamics tab's manual-K field (which then applies it). Works on any
// printer — it's slicer-side, not a firmware routine.

import { useRef, useState } from "react";
import { ModalBackdrop, ModalCloseButton } from "../ui/Modal";
import { usePopoverDismiss } from "../ui/usePopoverDismiss";
import type { FlowPaSlot } from "../printer/printerInstance";
import { driverErrorMessage, driverSendPaCalibration } from "./invokes";
import type { DriverId } from "./types";

export interface PaCalibrationDialogProps {
  rows: FlowPaSlot[];
  /** Human label for a slot row (brand · name · material·nozzle). */
  rowLabel: (row: FlowPaSlot) => string;
  instanceId: string;
  driverId: DriverId;
  /** Connection kind — drives the default K range's scale. */
  kind: string | undefined;
  /** Row to preselect (the checked/first row). */
  initialKey: string | null;
  onClose: () => void;
}

const rowKey = (s: FlowPaSlot): string => `${s.extruder_index}-${s.slot_index}`;

const swatchStyle = (color: string | null | undefined): React.CSSProperties =>
  color
    ? { background: color, border: "none" }
    : { background: "transparent", border: "1px dashed currentColor" };

/** The filament picker — same swatch + name + popover-of-slots as the material
 *  slot selector (`MaterialChip`), minus the material-index chip and routing
 *  semantics. Reuses that component's CSS + the shared popover-dismiss hook. */
function PaSlotSelect({
  rows,
  selectedKey,
  rowLabel,
  onPick,
}: {
  rows: FlowPaSlot[];
  selectedKey: string;
  rowLabel: (row: FlowPaSlot) => string;
  onPick: (key: string) => void;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  usePopoverDismiss(wrapRef, () => setOpen(false), open);
  const selected = rows.find((r) => rowKey(r) === selectedKey) ?? rows[0];

  return (
    <div className="config-chip-wrap" ref={wrapRef}>
      <button
        type="button"
        className="pacd-slot-chip"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <span className="fil-swatch" style={swatchStyle(selected?.color)} />
        <span className="fil-label">{selected ? rowLabel(selected) : "—"}</span>
        <svg
          className="pacd-chevron"
          width="9"
          height="9"
          viewBox="0 0 10 10"
          fill="none"
          aria-hidden
        >
          <path
            d="M2 4l3 3 3-3"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </button>
      {open && (
        <div
          className="printer-picker-menu material-menu"
          role="menu"
          onClick={(e) => e.stopPropagation()}
        >
          {rows.map((r) => {
            const k = rowKey(r);
            return (
              <button
                key={k}
                type="button"
                className={`ptpm-item ptpm-row${k === selectedKey ? " active" : ""}`}
                onClick={() => {
                  onPick(k);
                  setOpen(false);
                }}
              >
                <span className="ptpm-name">
                  <span className="ptpm-swatch" style={swatchStyle(r.color)} />
                  {rowLabel(r)}
                </span>
                <span className="ptpm-detail">{r.nozzle}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

const round4 = (n: number): number => Math.round(n * 1e4) / 1e4;

/** Default K sweep, pre-filled and editable. Keyed on firmware K-scale
 *  (Klipper `pressure_advance` seconds vs Bambu/Marlin `M900 K`) and scaled
 *  mildly by nozzle size (bigger nozzle → more flow → higher K). Direct-drive
 *  baseline; widen the range by hand for a bowden extruder. */
function defaultRange(
  kind: string | undefined,
  nozzle: string,
): { start: string; end: string; step: string } {
  const klipper = kind === "u1" || kind === "moonraker";
  let end = klipper ? 0.08 : 0.05;
  let step = 0.005;
  const dia = Number(nozzle);
  if (Number.isFinite(dia) && dia > 0) {
    const scale = dia / 0.4;
    end = round4(end * scale);
    step = round4(step * scale);
  }
  return { start: "0", end: String(end), step: String(step) };
}

export function PaCalibrationDialog({
  rows,
  rowLabel,
  instanceId,
  driverId,
  kind,
  initialKey,
  onClose,
}: PaCalibrationDialogProps): React.JSX.Element {
  const [selectedKey, setSelectedKey] = useState<string>(
    initialKey ?? (rows[0] ? rowKey(rows[0]) : ""),
  );
  const selected = rows.find((r) => rowKey(r) === selectedKey) ?? rows[0];

  const [range, setRange] = useState(() =>
    defaultRange(kind, selected?.nozzle ?? "0.4"),
  );
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reset the range to the freshly-picked slot's default — a different slot can
  // have a different nozzle, so its default range legitimately differs. This
  // does drop any manual edits, which is the intended trade for a correct
  // per-nozzle default; re-edit after picking.
  const onPickSlot = (key: string): void => {
    setSelectedKey(key);
    const row = rows.find((r) => rowKey(r) === key);
    setRange(defaultRange(kind, row?.nozzle ?? "0.4"));
  };

  const start = Number(range.start);
  const end = Number(range.end);
  const step = Number(range.step);
  const finite = [start, end, step].every(Number.isFinite);
  const valid =
    finite && start >= 0 && step > 0 && end > start + step && !!selected;

  const send = async (): Promise<void> => {
    if (!valid || !selected) return;
    setSending(true);
    setError(null);
    try {
      await driverSendPaCalibration(
        driverId,
        instanceId,
        selected.extruder_index,
        selected.slot_index,
        start,
        end,
        step,
      );
      onClose();
    } catch (e) {
      setError(driverErrorMessage(e));
    } finally {
      setSending(false);
    }
  };

  return (
    <ModalBackdrop
      onDismiss={onClose}
      cardClassName="cfd-card"
      backdropClassName="cfd-backdrop"
      ariaLabelledBy="pacd-title"
    >
      <header className="psm-header">
        <div className="psm-header-mark" data-brand="filament">
          <span>📏</span>
        </div>
        <div className="psm-header-text">
          <h2 id="pacd-title">Print PA calibration test</h2>
          <p>Sweeps pressure advance across a printed line test — read the best line by eye.</p>
        </div>
        <ModalCloseButton onClick={onClose} />
      </header>

      <div className="cfd-body">
        <div className="cfd-field">
          <span>Filament</span>
          <PaSlotSelect
            rows={rows}
            selectedKey={selectedKey}
            rowLabel={rowLabel}
            onPick={onPickSlot}
          />
        </div>
        <div className="pacd-range">
          <label className="cfd-field">
            <span>Start K</span>
            <input
              type="text"
              inputMode="decimal"
              value={range.start}
              onChange={(e) => setRange((r) => ({ ...r, start: e.target.value }))}
            />
          </label>
          <label className="cfd-field">
            <span>End K</span>
            <input
              type="text"
              inputMode="decimal"
              value={range.end}
              onChange={(e) => setRange((r) => ({ ...r, end: e.target.value }))}
            />
          </label>
          <label className="cfd-field">
            <span>Step</span>
            <input
              type="text"
              inputMode="decimal"
              value={range.step}
              onChange={(e) => setRange((r) => ({ ...r, step: e.target.value }))}
            />
          </label>
        </div>
        {error && (
          <div className="sp-error" role="alert">
            {error}
          </div>
        )}
      </div>

      <footer className="cfd-foot">
        <button type="button" className="apm-btn" onClick={onClose}>
          Cancel
        </button>
        <button
          type="button"
          className="apm-btn primary"
          onClick={() => void send()}
          disabled={!valid || sending}
          title={
            valid ? "Slice and send the test print" : "Enter a valid K range (end > start + step)"
          }
        >
          {sending ? "Sending…" : "Print test"}
        </button>
      </footer>
    </ModalBackdrop>
  );
}
