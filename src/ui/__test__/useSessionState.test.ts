// Store-semantics self-check: a stored falsy value ("" / false / 0) must win
// over the initial — presence, not truthiness, decides the fallback.

import { describe, expect, it } from "vitest";
import { readSession, writeSession } from "../useSessionState";

describe("session store", () => {
  it("falls back to initial only when the key was never written", () => {
    expect(readSession("test.unwritten", "initial")).toBe("initial");
  });

  it("stored falsy values win over the initial", () => {
    writeSession("test.search", "");
    writeSession("test.toggle", false);
    expect(readSession("test.search", "fallback")).toBe("");
    expect(readSession("test.toggle", true)).toBe(false);
  });
});
