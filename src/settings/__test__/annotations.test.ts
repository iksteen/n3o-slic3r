// Annotation catalog + support-toggle commit-shape tests (PR-4-12).

import { describe, expect, it } from "vitest";
import { ANNOTATIONS } from "../annotations/data";

describe("annotations catalog", () => {
  it("ships at least the ~30 highest-impact entries (Execution Plan §6 floor)", () => {
    expect(Object.keys(ANNOTATIONS).length).toBeGreaterThanOrEqual(30);
  });

  it("every entry is concise — capped at ~500 chars to keep tooltips readable", () => {
    for (const [key, text] of Object.entries(ANNOTATIONS)) {
      expect(text.length, `${key} annotation too long`).toBeLessThan(500);
    }
  });

  it("covers the canonical high-impact keys", () => {
    // Five "must have" keys the panel surfaces first on a default
    // A1 mini load. If any of these go missing the tooltip layer
    // loses its initial-impression value.
    const canonical = [
      "layer_height",
      "sparse_infill_density",
      "wall_loops",
      "enable_support",
      "nozzle_temperature",
    ];
    for (const key of canonical) {
      expect(ANNOTATIONS, `expected canonical key '${key}'`).toHaveProperty(key);
    }
  });
});
