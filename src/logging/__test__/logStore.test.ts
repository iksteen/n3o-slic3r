import { beforeEach, describe, expect, it } from "vitest";
import { clearLogs, getLogs, pushLog } from "../logStore";

describe("logStore", () => {
  beforeEach(() => clearLogs());

  it("appends entries with level and message", () => {
    pushLog("info", "ready");
    pushLog("error", "boom");
    const logs = getLogs();
    expect(logs).toHaveLength(2);
    expect(logs[0]).toMatchObject({ level: "info", msg: "ready" });
    expect(logs[1]).toMatchObject({ level: "error", msg: "boom" });
    expect(typeof logs[1].ts).toBe("number");
  });

  it("clears all entries", () => {
    pushLog("warn", "x");
    clearLogs();
    expect(getLogs()).toHaveLength(0);
  });

  it("caps the buffer, dropping the oldest entries", () => {
    for (let i = 0; i < 250; i++) pushLog("info", `m${i}`);
    const logs = getLogs();
    expect(logs).toHaveLength(200);
    // Oldest 50 dropped; the tail is the most recent.
    expect(logs[0].msg).toBe("m50");
    expect(logs[logs.length - 1].msg).toBe("m249");
  });

  it("returns a stable snapshot reference between mutations (useSyncExternalStore safety)", () => {
    pushLog("info", "a");
    const a = getLogs();
    // Same reference when nothing changed — a fresh array each call would
    // make useSyncExternalStore loop forever.
    expect(getLogs()).toBe(a);
    pushLog("info", "b");
    // New reference after a real mutation.
    expect(getLogs()).not.toBe(a);
  });

  it("clear on an empty store keeps the same (empty) reference", () => {
    clearLogs();
    const empty = getLogs();
    clearLogs();
    expect(getLogs()).toBe(empty);
  });
});
