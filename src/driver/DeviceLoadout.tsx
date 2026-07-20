// Filament loadout — what's physically loaded per the live driver report,
// projected into rows and rendered in the Devices monitor.

import type { AmsFilament, AmsTray, PrinterStatus } from "./types";
import { cssColorFromHex } from "./colorUtils";

export interface LoadoutSlot {
  key: string;
  label: string;
  color: string | null;
  name: string | null;
  material: string | null;
  /** The slot currently engaged at the nozzle / held by the grabber,
   *  per the live driver report — same derivation the AMS / toolhead
   *  strips use. */
  active: boolean;
}

/** Filament-name from an AMS/external spool's raw report: `"<type> <brand>"`
 *  (e.g. "PLA Basic"), or just the type, or null when untagged. */
function amsFilamentName(id: AmsFilament): string | null {
  const parts = [id.tray_type, id.sub_brand].filter((p): p is string => !!p);
  return parts.length > 0 ? parts.join(" ") : null;
}

function amsLoadoutRow(
  tray: AmsTray,
  unitId: number,
  multiUnit: boolean,
  activeSlot: number | null,
): LoadoutSlot {
  const id = tray.identity;
  return {
    key: `ams:${unitId}:${tray.id}`,
    label: multiUnit
      ? `${String.fromCharCode(65 + unitId)}:${tray.id + 1}`
      : `${tray.id + 1}`,
    color: id ? cssColorFromHex(id.color) : null,
    name: id ? amsFilamentName(id) : null,
    material: id ? id.tray_type : null,
    active: activeSlot != null && activeSlot === tray.id,
  };
}

/** Project the live driver report into loadout rows — what's *physically*
 *  loaded per the printer's MQTT. Intentionally decoupled from the plate /
 *  slot bindings (those are the slicing assignment, a separate concern the
 *  device panel never reflects). `[]` when offline / nothing reported yet,
 *  so a connection doesn't depend on the user having Synced slots. */
export function loadoutFromReport(status: PrinterStatus | null): LoadoutSlot[] {
  if (status == null) return [];
  const extra = status.extra;
  if (extra.kind === "U1") {
    return extra.data.toolhead_filaments.map((fil, i) => ({
      key: `th:${i}`,
      label: `T${i + 1}`,
      color: fil ? cssColorFromHex(fil.color) : null,
      name: fil ? fil.material_type : null,
      material: fil ? fil.material_type : null,
      active: extra.data.mounted_toolhead === i,
    }));
  }
  // Generic Moonraker/Klipper reports no AMS/toolhead loadout.
  if (extra.kind === "Moonraker") return [];
  const rows: LoadoutSlot[] = [];
  const { ams, external_spool } = extra.data;
  if (ams) {
    const multiUnit = ams.units.length > 1;
    for (const unit of ams.units) {
      for (const tray of unit.trays) {
        rows.push(amsLoadoutRow(tray, unit.id, multiUnit, ams.active_slot));
      }
    }
  }
  // The external spool slot always exists on the A1 mini, so its row
  // stays visible even when nothing is loaded — mirroring how the AMS
  // slots stay visible when empty. The backend populates
  // external_spool only when the external is the engaged tray
  // (tray_now == 254), so its presence == loaded == active.
  rows.push({
    key: "ext",
    label: "Ext",
    color: external_spool ? cssColorFromHex(external_spool.color) : null,
    name: external_spool ? amsFilamentName(external_spool) : null,
    material: external_spool ? external_spool.tray_type : null,
    active: external_spool != null,
  });
  return rows;
}

export function LoadoutPanel({
  offline,
  slots,
}: {
  offline: boolean;
  slots: LoadoutSlot[];
}): React.JSX.Element {
  if (offline) {
    return (
      <div className="device-loadout">
        <div className="device-loadout-header">
          <span>Filament loadout</span>
        </div>
        <div className="device-loadout-empty dim">Unknown — printer offline.</div>
      </div>
    );
  }
  const loaded = slots.filter((s) => s.name != null);
  if (slots.length === 0) {
    return (
      <div className="device-loadout">
        <div className="device-loadout-header">
          <span>Filament loadout</span>
        </div>
        <div className="device-loadout-empty dim">No filament reported.</div>
      </div>
    );
  }
  return (
    <div className="device-loadout">
      <div className="device-loadout-header">
        <span>Filament loadout</span>
        <span className="device-loadout-count">
          {loaded.length}/{slots.length}
        </span>
      </div>
      <div className="device-loadout-list">
        {slots.map((s) => (
          <div
            key={s.key}
            className={`device-loadout-row${s.name ? "" : " empty"}${
              s.active && s.name ? " active" : ""
            }`}
            title={
              s.name
                ? `${s.label} · ${s.name}${s.material ? ` (${s.material})` : ""}${
                    s.active ? " — currently engaged" : ""
                  }`
                : `${s.label} — empty`
            }
          >
            <span
              className="device-loadout-swatch"
              style={{ background: s.color ?? "transparent" }}
            />
            <span className="device-loadout-slot">{s.label}</span>
            <span className="device-loadout-name">{s.name ?? "Empty"}</span>
            <span className="device-loadout-mat">{s.material ?? "—"}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
