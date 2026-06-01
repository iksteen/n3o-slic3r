// PR-5-3 plate-tab view-model projection tests.
//
// The full hook lifecycle (Tauri `listen` + `invoke`) needs a
// browser-ish env we don't have set up in vitest. The interesting
// logic — the snapshot → tab-view projection — is pure, so that's
// what we exercise. The hook's event subscription is a thin
// `for-of listen()` over `PLATE_TAB_EVENT_NAMES`; if it drifts we'd
// notice via a vitest-run failure on the constant's identity tests
// below.

import { describe, expect, it } from "vitest";
import { projectSnapshot, PLATE_TAB_EVENT_NAMES } from "../usePlateTabs";
import type {
  PlateSnapshot,
  SceneObject,
  SceneSnapshot,
} from "../../viewport/types";


function plateSnap(opts: {
  id: number;
  name?: string;
  printer_identity?: string | null;
  objects?: number;
}): PlateSnapshot {
  const obj = (id: number): SceneObject => ({
    id,
    mesh: 1,
    transform: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
    name: `obj-${id}`,
    visible: true,
    extruder_id: null,
    group_id: null,
    parent: null,
  });
  return {
    plate_id: opts.id,
    name: opts.name ?? `Plate ${opts.id}`,
    metadata: { composition_order: opts.id },
    printer_identity: opts.printer_identity ?? null,
    printer_instance_id: null,
    material_to_slot: {},
    project_overrides: {},
    quality_profile: null,
    objects: Array.from({ length: opts.objects ?? 0 }, (_, i) => obj(i + 1)),
    selection: [],
    build_plate: null,
    exclusion_zones: [],
    bed: null,
    object_overrides: {},
    group_names: {},
  };
}

function snap(plates: PlateSnapshot[], activeId: number): SceneSnapshot {
  return {
    project_uuid: "test",
    source_path: null,
    user_overrides: {},
    file_metadata: {},
    meshes: [],
    plates,
    active_plate_id: activeId,
  };
}

describe("projectSnapshot", () => {
  it("maps every plate to {id, name, printerLabel, objectCount}", () => {
    const result = projectSnapshot(
      snap(
        [
          plateSnap({
            id: 1,
            name: "Body",
            printer_identity: "bambu_a1_mini",
            objects: 3,
          }),
          plateSnap({
            id: 2,
            name: "Supports",
            printer_identity: null,
            objects: 0,
          }),
        ],
        1,
      ),
    );
    expect(result.plates).toEqual([
      { id: 1, name: "Body", printerLabel: "bambu_a1_mini", objectCount: 3 },
      { id: 2, name: "Supports", printerLabel: null, objectCount: 0 },
    ]);
    expect(result.activePlateId).toBe(1);
    expect(result.loading).toBe(false);
  });

  it("preserves the snapshot's declaration order", () => {
    const result = projectSnapshot(
      snap(
        [
          plateSnap({ id: 7 }),
          plateSnap({ id: 3 }),
          plateSnap({ id: 5 }),
        ],
        3,
      ),
    );
    expect(result.plates.map((p) => p.id)).toEqual([7, 3, 5]);
    expect(result.activePlateId).toBe(3);
  });

  it("renders a non-default plate name verbatim (PR-5-3 rename)", () => {
    const result = projectSnapshot(
      snap([plateSnap({ id: 1, name: "Calibration tower" })], 1),
    );
    expect(result.plates[0].name).toBe("Calibration tower");
  });
});

describe("PLATE_TAB_EVENT_NAMES", () => {
  it("includes every plate-affecting channel the strip needs to track", () => {
    // If any of these go missing the strip silently goes stale — pin
    // them. Adding new ones to the constant is fine; this test only
    // guards the floor.
    const required = [
      "scene:plate_added",
      "scene:plate_removed",
      "scene:active_plate_changed",
      "scene:plate_metadata_changed",
      "scene:object_added",
      "scene:object_removed",
      "scene:bed_changed",
      "project:loaded",
    ];
    for (const name of required) {
      expect(PLATE_TAB_EVENT_NAMES).toContain(name);
    }
  });
});
