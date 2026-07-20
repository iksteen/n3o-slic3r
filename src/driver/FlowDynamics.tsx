// Flow Dynamics tab of the device page — per-filament pressure advance (K).
//
// One row per bound slot: brand, name, spool color, material·nozzle, and an
// editable K. K is keyed (filament identity × color × nozzle); an empty field
// falls back to the printer's material/nozzle default, shown as the input's
// placeholder. Two actions:
//   - Save: persist manually-edited K values (blank clears back to default).
//   - Calibrate selected: run FLOW_CALIBRATE on the printer for each checked
//     row, in sequence (the printer calibrates one toolhead at a time), and
//     store the measured K. Needs a connected driver; editing/saving doesn't.

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  flowPaValues,
  setCalibratedPa,
  type FlowPaSlot,
  type PrinterInstance,
} from "../printer/printerInstance";
import { useFilamentCatalog } from "../material/useFilamentCatalog";
import { driverErrorMessage } from "./invokes";
import {
  getInstanceCal,
  runCalibration,
  subscribe as subscribeCal,
} from "./paCalibrationStore";
import type { DriverId } from "./types";

interface FlowDynamicsProps {
  instance: PrinterInstance;
  driverId: DriverId | null;
  /** Printer is idle — a precondition for calibration (FLOW_CALIBRATE heats,
   *  purges and moves the toolhead; running it mid-print would wreck the job).
   *  Manual K editing/saving is unaffected. */
  printerIdle: boolean;
}

const rowKey = (s: { extruder_index: number; slot_index: number }): string =>
  `${s.extruder_index}-${s.slot_index}`;

const fmtK = (k: number | null): string => (k == null ? "" : String(k));

export function FlowDynamics({
  instance,
  driverId,
  printerIdle,
}: FlowDynamicsProps): React.JSX.Element {
  const { byIdentity } = useFilamentCatalog();
  const [rows, setRows] = useState<FlowPaSlot[] | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  // In-progress K text per row, only present for edited rows.
  const [edits, setEdits] = useState<Record<string, string>>({});
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState(false);

  const instanceId = instance.id;

  // Calibration status lives in a module store keyed by instance id, so it
  // survives this component unmounting (tab or printer switch) and can't leak
  // between printers. `busy`/`cal` are that printer's live run.
  const { busy, rows: cal } = useSyncExternalStore(subscribeCal, () =>
    getInstanceCal(instanceId),
  );

  // On-printer calibration is supported for the U1 (FLOW_CALIBRATE) and Bambu
  // (Flow-Dynamics auto-cali over MQTT). The Ender (plain Moonraker) has no such
  // routine. Bambu is gated to single-extruder (dual-nozzle H2D unsupported);
  // multi-AMS is allowed (addressed correctly, though only the A1 mini was
  // hardware-tested). Every printer still gets the view + manual K editing.
  const kind = instance.connection?.kind;
  const isBambu = kind === "bambu" && instance.extruders.length === 1;
  const canCalibrate = kind === "u1" || isBambu;

  const load = useCallback(async (): Promise<void> => {
    try {
      setRows(await flowPaValues(instanceId));
      setLoadErr(null);
    } catch (e) {
      setLoadErr(String(e));
    }
  }, [instanceId]);

  useEffect(() => {
    void load();
  }, [load]);

  // When a run finishes (busy true → false), pull the freshly-stored K into
  // the editable column and drop stale manual edits on the calibrated rows.
  const prevBusy = useRef(busy);
  useEffect(() => {
    if (prevBusy.current && !busy) {
      setEdits((prev) => {
        const next = { ...prev };
        for (const [k, c] of Object.entries(cal)) {
          if (c.phase === "done") delete next[k];
        }
        return next;
      });
      setChecked(new Set());
      void load();
    }
    prevBusy.current = busy;
  }, [busy, cal, load]);

  if (loadErr) {
    return (
      <div className="flow-dyn">
        <div className="sp-error" role="alert">
          {loadErr}
        </div>
      </div>
    );
  }
  if (rows == null) {
    return <div className="flow-dyn dim">Loading…</div>;
  }
  if (rows.length === 0) {
    return (
      <div className="flow-dyn dim">
        No filament loaded. Assign filaments to slots to tune their pressure
        advance.
      </div>
    );
  }

  const editValue = (row: FlowPaSlot): string => {
    const k = rowKey(row);
    if (k in edits) return edits[k];
    // While a run is active, show a just-measured K straight from the store —
    // the persisted value only lands in `current_k` after the end-of-cycle
    // park finishes, so this bridges that gap. Once the run ends, `busy` goes
    // false and the reload makes `current_k` authoritative again.
    if (busy) {
      const c = cal[k];
      if (c?.phase === "done" && c.k != null) return String(c.k);
    }
    return fmtK(row.current_k);
  };
  const isDirty = (row: FlowPaSlot): boolean => {
    const k = rowKey(row);
    return k in edits && edits[k].trim() !== fmtK(row.current_k).trim();
  };
  const isInvalid = (row: FlowPaSlot): boolean => {
    const t = editValue(row).trim();
    return t !== "" && !Number.isFinite(Number(t));
  };
  const dirtyRows = rows.filter(isDirty);
  const anyInvalid = rows.some(isInvalid);

  const toggle = (k: string): void =>
    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });

  const onSave = async (): Promise<void> => {
    setSaving(true);
    try {
      for (const row of dirtyRows) {
        if (isInvalid(row)) continue;
        const t = editValue(row).trim();
        const val = t === "" ? null : Number(t);
        await setCalibratedPa(
          instanceId,
          row.identity,
          row.color,
          row.nozzle,
          val,
        );
      }
      setEdits({});
      await load();
    } catch (e) {
      setLoadErr(driverErrorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  const onCalibrate = (): void => {
    if (driverId == null || !printerIdle) return;
    const targets = rows
      .filter((r) => checked.has(rowKey(r)))
      .map((r) => ({
        key: rowKey(r),
        extruderIndex: r.extruder_index,
        slotIndex: r.slot_index,
      }));
    // Fire-and-forget: the store owns the loop so it survives a remount. The
    // busy → false effect refreshes stored K when it finishes.
    void runCalibration(instanceId, driverId, targets, isBambu);
  };

  const calDone = Object.values(cal).filter(
    (c) => c.phase === "done" || c.phase === "error",
  ).length;
  const calTotal = Object.keys(cal).length;

  const renderStatus = (row: FlowPaSlot): React.JSX.Element | null => {
    const c = cal[rowKey(row)];
    if (!c) return null;
    switch (c.phase) {
      case "queued":
        return <span className="flow-dyn-stat dim">queued</span>;
      case "running":
        return (
          <span className="flow-dyn-stat running">
            <span className="flow-dyn-spinner" /> calibrating…
          </span>
        );
      case "done":
        return (
          <span className="flow-dyn-stat done">✓ K {fmtK(c.k ?? null)}</span>
        );
      case "error":
        return (
          <span className="flow-dyn-stat error" title={c.message}>
            ⚠ failed
          </span>
        );
    }
  };

  return (
    <div className="flow-dyn">
      <div className="flow-dyn-actions">
        <button
          className="device-ctl"
          onClick={() => void onSave()}
          disabled={busy || saving || dirtyRows.length === 0 || anyInvalid}
          type="button"
          title="Persist manually-edited K values"
        >
          {saving ? "Saving…" : "Save"}
        </button>
        {canCalibrate && (
          <button
            className="device-ctl primary"
            onClick={onCalibrate}
            disabled={
              busy || driverId == null || !printerIdle || checked.size === 0
            }
            type="button"
            title={
              driverId == null
                ? "Connect the printer to calibrate"
                : !printerIdle
                  ? "Printer must be idle to calibrate"
                  : isBambu
                    ? "Auto-calibrate the checked filaments (one printer job)"
                    : "Run FLOW_CALIBRATE for the checked filaments, in sequence"
            }
          >
            {busy ? "Calibrating…" : `Calibrate selected (${checked.size})`}
          </button>
        )}
        {canCalibrate && busy && (
          <span className="flow-dyn-progress dim">
            calibrating {calDone}/{calTotal} — this takes a few minutes each
          </span>
        )}
        {canCalibrate && driverId == null && !busy && (
          <span className="flow-dyn-progress dim">
            printer offline — edit and save K manually, or connect to calibrate
          </span>
        )}
        {canCalibrate && driverId != null && !printerIdle && !busy && (
          <span className="flow-dyn-progress dim">
            printer busy — calibration needs an idle printer; edit and save K
            manually meanwhile
          </span>
        )}
      </div>

      <table className="flow-dyn-table">
        <thead>
          <tr>
            {canCalibrate && <th />}
            <th>Filament</th>
            <th>Material</th>
            <th>
              K{" "}
              <span className="dim" style={{ fontWeight: 400 }}>
                (blank = default)
              </span>
            </th>
            {canCalibrate && <th />}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => {
            const k = rowKey(row);
            const summary = byIdentity.get(row.identity);
            const brand = summary?.vendor ?? "";
            const name = summary?.display_name ?? row.identity;
            const material = summary?.base_type ?? "?";
            return (
              <tr key={k}>
                {canCalibrate && (
                  <td>
                    <input
                      type="checkbox"
                      checked={checked.has(k)}
                      onChange={() => toggle(k)}
                      disabled={busy}
                      aria-label={`Select ${name} for calibration`}
                    />
                  </td>
                )}
                <td>
                  <div className="flow-dyn-fil">
                    <span
                      className="flow-dyn-swatch"
                      style={
                        row.color ? { background: row.color } : undefined
                      }
                      title={row.color || "no color set"}
                    />
                    <span className="flow-dyn-fil-names">
                      <span className="flow-dyn-name">{name}</span>
                      {brand && (
                        <span className="flow-dyn-brand dim">{brand}</span>
                      )}
                    </span>
                  </div>
                </td>
                <td className="flow-dyn-mat">
                  {material} · {row.nozzle}
                </td>
                <td>
                  <input
                    className={`flow-dyn-k${isInvalid(row) ? " invalid" : ""}`}
                    type="text"
                    inputMode="decimal"
                    value={editValue(row)}
                    placeholder={
                      row.default_k == null ? "—" : String(row.default_k)
                    }
                    onChange={(e) =>
                      setEdits((prev) => ({ ...prev, [k]: e.target.value }))
                    }
                    disabled={busy}
                  />
                </td>
                {canCalibrate && <td>{renderStatus(row)}</td>}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
