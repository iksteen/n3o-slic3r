// Device-panel loadout projection tests.
//
// The device panel shows what's *physically* loaded per the live MQTT
// report — intentionally decoupled from the instance's synced slot
// bindings (the slicing assignment). `loadoutFromReport` is the pure
// projection; these exercise it without rendering DOM.

import { describe, expect, it } from "vitest";
import { loadoutFromReport } from "../DevicesView";
import type { DriverExtra, PrinterStatus } from "../types";

function status(extra: DriverExtra): PrinterStatus {
  return {
    connection: { state: "Connected" },
    job: null,
    temps: { nozzles: [], bed: { current: 0, target: 0 }, chamber: null },
    extra,
    last_updated: 0,
  };
}

const amsFil = (tray_type: string, color: string, sub_brand: string | null = null) => ({
  tray_type,
  color,
  sub_brand,
  multi_colors: [],
  filament_id: null,
});

describe("loadoutFromReport", () => {
  it("returns [] for an offline/null status (no Sync needed, but nothing to show)", () => {
    expect(loadoutFromReport(null)).toEqual([]);
  });

  it("projects Bambu AMS trays + external spool straight from the report", () => {
    const rows = loadoutFromReport(
      status({
        kind: "Bambu",
        data: {
          mounted_plate: null,
          current_stage: null,
          print_error_code: null,
          fan_speed: null,
          ams: {
            active_slot: 2,
            units: [
              {
                id: 0,
                trays: [
                  { id: 0, identity: amsFil("PLA", "FF0000FF", "Basic") },
                  { id: 1, identity: null },
                  { id: 2, identity: amsFil("PETG", "00FF00FF") },
                  { id: 3, identity: null },
                ],
              },
            ],
          },
          external_spool: amsFil("TPU", "0000FFFF"),
        },
      }),
    );
    expect(rows).toHaveLength(5); // 4 trays + external
    expect(rows[0]).toMatchObject({
      label: "1",
      color: "#FF0000",
      name: "PLA Basic",
      material: "PLA",
      active: false,
    });
    expect(rows[1]).toMatchObject({ label: "2", color: null, name: null, material: null });
    // active_slot 2 → tray id 2 highlighted; name falls back to type when no brand.
    expect(rows[2]).toMatchObject({ label: "3", name: "PETG", material: "PETG", active: true });
    expect(rows[4]).toMatchObject({ label: "Ext", color: "#0000FF", material: "TPU", active: false });
  });

  it("omits the external spool row when none is reported", () => {
    const rows = loadoutFromReport(
      status({
        kind: "Bambu",
        data: {
          mounted_plate: null,
          current_stage: null,
          print_error_code: null,
          fan_speed: null,
          ams: { active_slot: null, units: [{ id: 0, trays: [{ id: 0, identity: amsFil("PLA", "FF0000FF") }] }] },
          external_spool: null,
        },
      }),
    );
    expect(rows.map((r) => r.label)).toEqual(["1"]);
  });

  it("projects U1 toolhead filaments, with the mounted toolhead active", () => {
    const rows = loadoutFromReport(
      status({
        kind: "U1",
        data: {
          mounted_toolhead: 1,
          toolhead_filaments: [
            { material_type: "PLA", color: "FF0000FF" },
            { material_type: "PETG", color: "00FF00FF" },
            null,
          ],
          current_stage: null,
          fan_speed: null,
        },
      }),
    );
    expect(rows).toHaveLength(3);
    expect(rows[0]).toMatchObject({ label: "T1", color: "#FF0000", material: "PLA", active: false });
    expect(rows[1]).toMatchObject({ label: "T2", color: "#00FF00", material: "PETG", active: true });
    expect(rows[2]).toMatchObject({ label: "T3", color: null, name: null, active: false });
  });
});
