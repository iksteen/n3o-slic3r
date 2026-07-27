import { describe, expect, it } from "vitest";
import { printerFree } from "../devicesStatus";

describe("printerFree", () => {
  it("allows action when stopped, including after a failed/cancelled print", () => {
    expect(printerFree("idle")).toBe(true);
    expect(printerFree("error")).toBe(true);
  });

  it("blocks while a job owns the machine, or when the link is down", () => {
    expect(printerFree("preparing")).toBe(false);
    expect(printerFree("printing")).toBe(false);
    expect(printerFree("paused")).toBe(false);
    expect(printerFree("offline")).toBe(false);
  });
});
