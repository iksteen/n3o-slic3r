import { describe, expect, it } from "vitest";
import {
  isEditEvent,
  isSavedEvent,
  PLATE_EDIT_EVENTS,
  PROJECT_WIDE_EDIT_EVENT,
} from "../editEvents";

describe("editEvents classification", () => {
  it("plate-scoped content edits + the project-wide edit are edits", () => {
    for (const name of PLATE_EDIT_EVENTS) expect(isEditEvent(name)).toBe(true);
    expect(isEditEvent(PROJECT_WIDE_EDIT_EVENT)).toBe(true);
  });

  it("selection, navigation, and warnings are NOT edits", () => {
    for (const name of [
      "scene:selection_changed",
      "scene:active_plate_changed",
      "scene:plate_added",
      "scene:plate_removed",
      "scene:object_out_of_bounds",
      "scene:non_uniform_scale",
      "scene:mesh_loaded",
    ]) {
      expect(isEditEvent(name)).toBe(false);
    }
  });

  it("save / load / import return to a clean baseline", () => {
    for (const name of ["project:saved", "project:loaded", "project:imported"]) {
      expect(isSavedEvent(name)).toBe(true);
      expect(isEditEvent(name)).toBe(false);
    }
  });
});
