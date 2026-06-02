import { describe, expect, it } from "vitest";

import { isObjectOverridable, type OptScopeFlags } from "../types";

const scope = (s: Partial<OptScopeFlags>): OptScopeFlags => ({
  project: false,
  object: false,
  region: false,
  ...s,
});

describe("isObjectOverridable (Object-tab editability gate)", () => {
  it("allows object-scoped settings", () => {
    expect(isObjectOverridable(scope({ object: true }))).toBe(true);
  });

  it("allows region-scoped settings", () => {
    expect(isObjectOverridable(scope({ region: true }))).toBe(true);
  });

  it("allows settings that are both object and project scope", () => {
    expect(isObjectOverridable(scope({ object: true, project: true }))).toBe(
      true,
    );
  });

  it("disables project/print-only settings", () => {
    expect(isObjectOverridable(scope({ project: true }))).toBe(false);
  });

  it("disables dangling no-scope options (e.g. ironing_expansion) so the UI never authors an override the slicer drops", () => {
    // No scope bit set at all — the exact case that slipped past the old
    // `project && !object && !region` gate.
    expect(isObjectOverridable(scope({}))).toBe(false);
  });
});
