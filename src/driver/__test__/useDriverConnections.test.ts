// Tests for the auto-connection reconciler's pure helpers.
//
// The hook itself wires module-scoped state to Tauri commands; that
// path needs an integration test with mocked invokes (out of scope
// for this v1 sweep). What we pin here is the decision logic that
// drives every register / unregister / replace call: the validity
// check that gates whether an instance gets a driver at all, and
// the diff that turns desired vs current state into an action list.

import { beforeEach, describe, expect, it } from "vitest";
import {
  connectionSignature,
  configForConnection,
  isConnectionUsable,
  resetDriverConnectionsForTests,
  seedReconcilerStateForTests,
  summaryForTests,
} from "../useDriverConnections";
import type { ConnectionInfo } from "../../printer/printerInstance";

const bambu = (overrides: Partial<{
  host: string;
  access_code: string;
}> = {}): ConnectionInfo => ({
  kind: "bambu",
  host: "192.168.1.42",
  access_code: "12345678",
  ...overrides,
});

const u1 = (overrides: Partial<{ host: string; port: number }> = {}): ConnectionInfo => ({
  kind: "u1",
  host: "snappy.local",
  port: 80,
  ...overrides,
});

describe("isConnectionUsable", () => {
  it("rejects a null connection", () => {
    expect(isConnectionUsable(null)).toBe(false);
  });

  it("requires both host and access_code for Bambu", () => {
    expect(isConnectionUsable(bambu())).toBe(true);
    expect(isConnectionUsable(bambu({ host: "" }))).toBe(false);
    expect(isConnectionUsable(bambu({ host: "   " }))).toBe(false);
    expect(isConnectionUsable(bambu({ access_code: "" }))).toBe(false);
    expect(isConnectionUsable(bambu({ access_code: "  " }))).toBe(false);
  });

  it("requires host + a valid port for U1", () => {
    expect(isConnectionUsable(u1())).toBe(true);
    expect(isConnectionUsable(u1({ host: "" }))).toBe(false);
    expect(isConnectionUsable(u1({ port: 0 }))).toBe(false);
    expect(isConnectionUsable(u1({ port: 65536 }))).toBe(false);
  });
});

describe("connectionSignature", () => {
  it("returns a stable 'none' for null", () => {
    expect(connectionSignature(null)).toBe("none");
  });

  it("uses different signatures across kinds", () => {
    // Even with the same host, Bambu and U1 are different drivers.
    expect(connectionSignature(bambu({ host: "x" }))).not.toBe(
      connectionSignature(u1({ host: "x" })),
    );
  });

  it("treats trimmed and untrimmed host/access-code as identical", () => {
    expect(connectionSignature(bambu({ host: "  192.168.1.42  " }))).toBe(
      connectionSignature(bambu({ host: "192.168.1.42" })),
    );
  });

  it("changes when port changes", () => {
    expect(connectionSignature(u1({ port: 80 }))).not.toBe(
      connectionSignature(u1({ port: 8080 })),
    );
  });
});

describe("configForConnection", () => {
  it("emits PascalCase 'Bambu' kind matching the Rust DriverConfig wire shape", () => {
    expect(configForConnection(bambu())).toEqual({
      kind: "Bambu",
      data: {
        host: "192.168.1.42",
        access_code: "12345678",
      },
    });
  });

  it("emits PascalCase 'U1' kind with port", () => {
    expect(configForConnection(u1({ port: 8080 }))).toEqual({
      kind: "U1",
      data: {
        host: "snappy.local",
        port: 8080,
      },
    });
  });

  it("trims host + access_code on the way out", () => {
    expect(
      configForConnection(
        bambu({ host: "  192.168.1.42  ", access_code: "  87654321  " }),
      ),
    ).toEqual({
      kind: "Bambu",
      data: {
        host: "192.168.1.42",
        access_code: "87654321",
      },
    });
  });
});

describe("summaryFor (picker-chip status derivation)", () => {
  beforeEach(() => {
    resetDriverConnectionsForTests();
  });

  it("returns `none` when the saved connection isn't usable", () => {
    expect(summaryForTests("printer-x", null).status).toBe("none");
    expect(summaryForTests("printer-x", bambu({ host: "" })).status).toBe(
      "none",
    );
  });

  it("returns `connecting` (with driverId) when LIVE has no runtime status yet", () => {
    // The reconciler placed the driver in LIVE but the bootstrap
    // `driverStatus` call hasn't landed — treat as still
    // connecting so the dot doesn't pre-emptively turn green.
    const conn = bambu();
    seedReconcilerStateForTests({
      live: [
        ["printer-x", { id: 42, signature: connectionSignature(conn) }],
      ],
    });
    const s = summaryForTests("printer-x", conn);
    expect(s.status).toBe("connecting");
    expect(s.driverId).toBe(42);
  });

  it("returns `connected` when runtime state is Connected", () => {
    const conn = bambu();
    seedReconcilerStateForTests({
      live: [
        ["printer-x", { id: 42, signature: connectionSignature(conn) }],
      ],
      runtimeStatus: [["printer-x", { state: "Connected" }]],
    });
    const s = summaryForTests("printer-x", conn);
    expect(s.status).toBe("connected");
    expect(s.driverId).toBe(42);
  });

  it("maps runtime Connecting / Reconnecting to `connecting`", () => {
    const conn = bambu();
    seedReconcilerStateForTests({
      live: [
        ["a", { id: 1, signature: connectionSignature(conn) }],
        ["b", { id: 2, signature: connectionSignature(conn) }],
      ],
      runtimeStatus: [
        ["a", { state: "Connecting" }],
        ["b", { state: "Reconnecting", data: { in_seconds: 5, reason: "boom" } }],
      ],
    });
    expect(summaryForTests("a", conn).status).toBe("connecting");
    expect(summaryForTests("b", conn).status).toBe("connecting");
  });

  it("maps runtime Disconnected to `failed` with the reason", () => {
    const conn = bambu();
    seedReconcilerStateForTests({
      live: [
        ["printer-x", { id: 42, signature: connectionSignature(conn) }],
      ],
      runtimeStatus: [
        [
          "printer-x",
          { state: "Disconnected", data: { reason: "host unreachable" } },
        ],
      ],
    });
    const s = summaryForTests("printer-x", conn);
    expect(s.status).toBe("failed");
    expect(s.reason).toBe("host unreachable");
    expect(s.driverId).toBe(42);
  });

  it("returns `connecting` when the reconciler is mid-flight", () => {
    seedReconcilerStateForTests({ inFlight: ["printer-x"] });
    expect(summaryForTests("printer-x", bambu()).status).toBe("connecting");
  });

  it("returns `failed` with the reason when the reconciler errored", () => {
    seedReconcilerStateForTests({
      failed: [["printer-x", "host unreachable"]],
    });
    const s = summaryForTests("printer-x", bambu());
    expect(s.status).toBe("failed");
    expect(s.reason).toBe("host unreachable");
  });

  it("treats `usable config, no LIVE, no in-flight, no failure` as connecting", () => {
    // This is the tiny gap between dep-key flip and effect run.
    expect(summaryForTests("printer-x", bambu()).status).toBe("connecting");
  });

  it("prefers IN_FLIGHT over FAILED when both are set for the same identity", () => {
    // F14: connect-failed rollback path clears IN_FLIGHT before
    // awaiting driverUnregister so the picker can demote to
    // "failed" immediately. But during a NEW register attempt
    // after a previous failure, IN_FLIGHT is set AND the prior
    // FAILED entry hasn't been cleared yet. The new attempt's
    // status should be "connecting" (the optimistic state), not
    // the stale "failed".
    seedReconcilerStateForTests({
      inFlight: ["printer-x"],
      failed: [["printer-x", "previous attempt: host unreachable"]],
    });
    const s = summaryForTests("printer-x", bambu());
    expect(s.status).toBe("connecting");
    expect(s.reason).toBe(null);
  });

  it("returns `connecting` when the LIVE entry's signature is stale vs the saved connection", () => {
    // User just saved new credentials; reconciler hasn't replaced
    // the LIVE entry yet. Picker should DEMOTE from connected to
    // connecting so it stops pointing at the soon-to-be-killed
    // driver.
    const oldConn = bambu({ access_code: "11111111" });
    const newConn = bambu({ access_code: "22222222" });
    seedReconcilerStateForTests({
      live: [
        ["printer-x", { id: 42, signature: connectionSignature(oldConn) }],
      ],
      runtimeStatus: [["printer-x", { state: "Connected" }]],
    });
    const s = summaryForTests("printer-x", newConn);
    expect(s.status).toBe("connecting");
    // driverId still surfaces (so handlers can avoid the stale id
    // by branching on status), but status tells the truth.
    expect(s.driverId).toBe(42);
  });

  it("treats two different instance.id keys with the same vendor profile as independent", () => {
    // Two A1 minis (different UUIDs) — each gets its own LIVE
    // entry and its own summary. Without F1 these would collide
    // on `vendor_profile_ref` and only one would surface.
    const conn1 = bambu({ host: "10.0.0.1" });
    const conn2 = bambu({ host: "10.0.0.2" });
    seedReconcilerStateForTests({
      live: [
        ["a1m-uuid-1", { id: 1, signature: connectionSignature(conn1) }],
        ["a1m-uuid-2", { id: 2, signature: connectionSignature(conn2) }],
      ],
      runtimeStatus: [
        ["a1m-uuid-1", { state: "Connected" }],
        ["a1m-uuid-2", { state: "Disconnected", data: { reason: "x" } }],
      ],
    });
    expect(summaryForTests("a1m-uuid-1", conn1).driverId).toBe(1);
    expect(summaryForTests("a1m-uuid-1", conn1).status).toBe("connected");
    expect(summaryForTests("a1m-uuid-2", conn2).driverId).toBe(2);
    expect(summaryForTests("a1m-uuid-2", conn2).status).toBe("failed");
  });
});
