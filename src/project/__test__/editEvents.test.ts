import { describe, expect, it } from "vitest";
import { PROJECT_REPLACED_EVENTS } from "../editEvents";

describe("editEvents classification", () => {
  it("project replacement = load + import + undo/redo, but NOT save (slice artifacts)", () => {
    // Open / import / undo-redo swap the project wholesale → stale every
    // plate's slice artifacts (output, preview, tower). Save does not — the
    // slice stays valid — so it must be excluded from the invalidation set.
    expect([...PROJECT_REPLACED_EVENTS].sort()).toEqual([
      "project:imported",
      "project:loaded",
      "project:restored",
    ]);
    expect((PROJECT_REPLACED_EVENTS as readonly string[]).includes("project:saved")).toBe(
      false,
    );
  });
});
