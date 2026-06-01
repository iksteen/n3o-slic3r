// PR-5-9 — host projection + visibility-toggle persistence tests.
//
// The host renders SettingsPanel; the full render path needs a DOM
// (we don't have one set up). What's testable in pure form is the
// projection: snapshot → active plate → selected object → panel
// shape, plus the localStorage-backed visibility toggle.

import { describe, expect, it } from "vitest";
import {
  activePlate,
  allObjectsForPanel,
  selectedObject,
} from "../SettingsPanelHost";
import type { ProjectSession } from "../../project/useProjectSession";
import { SESSION_EVENT_NAMES } from "../../project/useProjectSession";
import type {
  PlateSnapshot,
  SceneObject,
  SceneSnapshot,
} from "../../viewport/types";


function obj(id: number, name: string): SceneObject {
  return {
    id,
    mesh: 1,
    transform: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
    name,
    visible: true,
    extruder_id: null,
    parent: null,
  };
}

function plate(opts: {
  id: number;
  objects?: SceneObject[];
  selection?: number[];
  object_overrides?: Record<number, Record<string, string>>;
}): PlateSnapshot {
  return {
    plate_id: opts.id,
    name: `Plate ${opts.id}`,
    metadata: { composition_order: opts.id },
    printer_identity: null,
    printer_instance_id: null,
    material_to_slot: {},
    project_overrides: {},
    quality_profile: null,
    objects: opts.objects ?? [],
    selection: opts.selection ?? [],
    build_plate: null,
    exclusion_zones: [],
    bed: null,
    object_overrides: opts.object_overrides ?? {},
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

function session(overrides: Partial<ProjectSession> = {}): ProjectSession {
  return {
    cascadeHandle: null,
    snapshot: null,
    loading: false,
    error: null,
    ...overrides,
  };
}

describe("activePlate", () => {
  it("returns null when the snapshot hasn't loaded", () => {
    expect(activePlate(session())).toBeNull();
  });

  it("finds the plate matching active_plate_id", () => {
    const s = session({
      snapshot: snap([plate({ id: 1 }), plate({ id: 2 })], 2),
    });
    expect(activePlate(s)?.plate_id).toBe(2);
  });

  it("returns null when active_plate_id refers to a missing plate (defensive)", () => {
    const s = session({
      snapshot: snap([plate({ id: 1 })], 99),
    });
    expect(activePlate(s)).toBeNull();
  });
});

describe("selectedObject", () => {
  it("returns null on a null plate", () => {
    expect(selectedObject(null)).toBeNull();
  });

  it("returns null when selection is empty", () => {
    expect(selectedObject(plate({ id: 1 }))).toBeNull();
  });

  it("projects the first selected object to {id, name}", () => {
    const p = plate({
      id: 1,
      objects: [obj(101, "Body"), obj(102, "Lid")],
      selection: [102, 101],
    });
    expect(selectedObject(p)).toEqual({ id: 102, name: "Lid" });
  });

  it("returns null when the selected id isn't on the plate (race protection)", () => {
    const p = plate({
      id: 1,
      objects: [obj(101, "Body")],
      selection: [999],
    });
    expect(selectedObject(p)).toBeNull();
  });
});

describe("allObjectsForPanel", () => {
  it("merges per-object overrides into each entry", () => {
    const p = plate({
      id: 1,
      objects: [obj(101, "Body"), obj(102, "Lid")],
      object_overrides: { 101: { layer_height: "0.12" } },
    });
    expect(allObjectsForPanel(p)).toEqual([
      {
        id: 101,
        name: "Body",
        color: null,
        overrides: { layer_height: "0.12" },
      },
      { id: 102, name: "Lid", color: null, overrides: {} },
    ]);
  });

  it("returns an empty list on a null plate", () => {
    expect(allObjectsForPanel(null)).toEqual([]);
  });
});

describe("SESSION_EVENT_NAMES", () => {
  it("covers everything the panel host needs (override channels + plate flow)", () => {
    // If any of these go missing the panel context goes stale —
    // pin the floor.
    const required = [
      "scene:plate_added",
      "scene:plate_removed",
      "scene:active_plate_changed",
      "scene:plate_metadata_changed",
      "scene:object_added",
      "scene:object_removed",
      "scene:object_updated",
      "scene:selection_changed",
      "scene:object_overrides_changed",
      "scene:project_overrides_changed",
      "scene:bed_changed",
      "project:loaded",
    ];
    for (const name of required) {
      expect(SESSION_EVENT_NAMES).toContain(name);
    }
  });
});
