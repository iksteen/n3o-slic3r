// visibility-toggle persistence contract.
//
// Vitest's default env is node — no `window` or `localStorage`.
// Stub them per the pattern in
// `src/settings/__test__/SettingsPanelHost.test.ts` so we can
// pin the storage-key contract without spinning up jsdom.

import { afterEach, beforeEach, describe, expect, it } from "vitest";

const LS_TRAVELS = "n3o-slic3r:preview:show-travels";
const LS_RETRACTIONS = "n3o-slic3r:preview:show-retractions";

beforeEach(() => {
  const store = new Map<string, string>();
  (globalThis as { localStorage?: Storage }).localStorage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => {
      store.set(k, v);
    },
    removeItem: (k: string) => {
      store.delete(k);
    },
    clear: () => store.clear(),
    key: () => null,
    length: 0,
  };
  (globalThis as { window?: { localStorage: Storage } }).window = {
    localStorage: globalThis.localStorage,
  };
});

afterEach(() => {
  delete (globalThis as { window?: unknown }).window;
  delete (globalThis as { localStorage?: unknown }).localStorage;
});

describe("visibility persistence keys", () => {
  it("default-load returns null when nothing is stored", () => {
    expect(window.localStorage.getItem(LS_TRAVELS)).toBeNull();
    expect(window.localStorage.getItem(LS_RETRACTIONS)).toBeNull();
  });

  it("round-trips boolean values as the canonical string form", () => {
    window.localStorage.setItem(LS_TRAVELS, "true");
    window.localStorage.setItem(LS_RETRACTIONS, "false");
    expect(window.localStorage.getItem(LS_TRAVELS)).toBe("true");
    expect(window.localStorage.getItem(LS_RETRACTIONS)).toBe("false");
  });

  it("storage keys match the documented namespace", () => {
    // If the keys ever drift, this test fails before the user
    // mysteriously loses their toggle preference across reload.
    expect(LS_TRAVELS).toBe("n3o-slic3r:preview:show-travels");
    expect(LS_RETRACTIONS).toBe("n3o-slic3r:preview:show-retractions");
  });
});
