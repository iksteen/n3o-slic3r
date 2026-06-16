// hotkeyInhibit re-entrancy self-check — the refcount must only clear when
// every owner has released, and a double-release must not underflow it.

import { describe, expect, it } from "vitest";
import { hotkeysInhibited, inhibitHotkeys } from "../hotkeyInhibit";

describe("inhibitHotkeys", () => {
  it("is re-entrant: stacked inhibits clear only when all release", () => {
    expect(hotkeysInhibited()).toBe(false);
    const a = inhibitHotkeys();
    expect(hotkeysInhibited()).toBe(true);
    const b = inhibitHotkeys();
    a();
    expect(hotkeysInhibited()).toBe(true); // b still holds
    b();
    expect(hotkeysInhibited()).toBe(false);
  });

  it("release is idempotent — a double-release can't underflow", () => {
    const a = inhibitHotkeys();
    const b = inhibitHotkeys();
    a();
    a(); // second call is a no-op
    expect(hotkeysInhibited()).toBe(true); // b still holds
    b();
    expect(hotkeysInhibited()).toBe(false);
  });
});
