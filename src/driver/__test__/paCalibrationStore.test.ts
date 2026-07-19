// Tests for the pressure-advance calibration store — the module-level state
// that survives FlowDynamics remounts. Pins the behaviour the two reported UI
// bugs turned on: per-row phase progression, busy lifecycle, per-instance
// isolation, and the concurrent-run guard.

import { beforeEach, describe, expect, it, vi } from "vitest";

// driverCalibratePa is the real dependency; resolve/reject it per test.
// driverParkExtruder is fired once at cycle end — mocked to a no-op spy.
const calibrate = vi.fn();
const park = vi.fn().mockResolvedValue(undefined);
vi.mock("../invokes", () => ({
  driverCalibratePa: (...args: unknown[]) => calibrate(...args),
  driverParkExtruder: (...args: unknown[]) => park(...args),
  driverErrorMessage: (e: unknown) => String(e),
}));

import {
  getInstanceCal,
  runCalibration,
  type CalTarget,
} from "../paCalibrationStore";

const target = (n: number): CalTarget => ({
  key: `0-${n}`,
  extruderIndex: 0,
  slotIndex: n,
});

describe("paCalibrationStore", () => {
  beforeEach(() => {
    calibrate.mockReset();
    park.mockClear();
  });

  it("walks each target queued → done and clears busy at the end", async () => {
    calibrate.mockResolvedValueOnce(0.021).mockResolvedValueOnce(0.033);
    await runCalibration("A", 1, [target(0), target(1)]);

    const state = getInstanceCal("A");
    expect(state.busy).toBe(false);
    expect(state.rows["0-0"]).toEqual({ phase: "done", k: 0.021 });
    expect(state.rows["0-1"]).toEqual({ phase: "done", k: 0.033 });
    expect(calibrate).toHaveBeenCalledTimes(2);
    // Cycle done → park the active toolhead exactly once.
    expect(park).toHaveBeenCalledTimes(1);
    expect(park).toHaveBeenCalledWith(1);
  });

  it("records a failed row as error without aborting the run", async () => {
    calibrate.mockRejectedValueOnce("not homed").mockResolvedValueOnce(0.02);
    await runCalibration("B", 1, [target(0), target(1)]);

    const state = getInstanceCal("B");
    expect(state.rows["0-0"]).toEqual({ phase: "error", message: "not homed" });
    expect(state.rows["0-1"]).toEqual({ phase: "done", k: 0.02 });
    expect(state.busy).toBe(false);
  });

  it("isolates state by instance id — no leak across printers", async () => {
    calibrate.mockResolvedValue(0.05);
    await runCalibration("C", 1, [target(0)]);

    expect(getInstanceCal("C").rows["0-0"]).toEqual({ phase: "done", k: 0.05 });
    // A printer with no run reads the shared empty snapshot.
    expect(getInstanceCal("D")).toEqual({ busy: false, rows: {} });
  });

  it("ignores a second run while one is already active", async () => {
    let release!: (k: number) => void;
    calibrate.mockImplementation(
      () => new Promise<number>((res) => (release = res)),
    );

    const first = runCalibration("E", 1, [target(0)]);
    expect(getInstanceCal("E").busy).toBe(true);

    // Second call while busy is a no-op — doesn't queue the extra target.
    await runCalibration("E", 1, [target(1)]);
    expect(getInstanceCal("E").rows["0-1"]).toBeUndefined();

    release(0.02);
    await first;
    expect(getInstanceCal("E").busy).toBe(false);
  });
});
