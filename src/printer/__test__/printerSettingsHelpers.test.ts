// PrinterSettingsModal pure-helper tests.
//
// Component rendering / submit lifecycle (React + Tauri) needs a
// jsdom + RTL setup we don't have (same pattern as
// `AddPrinterModal.test.tsx` and `PrinterCredentialsDialog.test.ts`).
// The helpers we pin here are the ones that drive the modal's
// observable behavior: driver branching, draft extraction from
// the instance, and the dirty-marker roll-up that gates the save
// button + the unsaved-changes discard overlay.

import { describe, expect, it } from "vitest";
import {
  computeChanged,
  computeSectionDirty,
  initialDraft,
  MACHINE_PAGE_ORDER,
  orderGroupsOtherLast,
  notesLast,
} from "../printerSettingsHelpers";
import {
  validateBambuConnection,
  validateU1Connection,
} from "../connectionValidation";
import type { ConnectionInfo, PrinterInstance } from "../printerInstance";
import { categorize } from "../../settings/nav/categories";
import type { OptionSummary } from "../../settings/types";

// `driverKindFor` was removed in F9 — driver_kind is authored in
// each printer's model.toml and carried through onto PrinterProfile
// by registry::hydrate_profile. The provenance is covered by
// `src-tauri/src/core/printer/registry.rs::tests::
// driver_kind_declared_in_model_toml`.

/** Minimal instance fixture used across the helpers. Defaults match
 *  the bambi bundled fixture closely (A1 mini, 1 AMS → 5 slots). */
function fixture(overrides: Partial<PrinterInstance> = {}): PrinterInstance {
  return {
    id: "test-id",
    display_name: "Test Printer",
    vendor_profile_ref: "bambu-lab-a1-mini",
    printer_fragment_slug: "bambu-lab-a1-mini",
    default_filament_fragment_slug: "bambu-pla-basic",
    quality_profile: "0.20mm-standard",
    connection: null,
    extruders: [
      {
        installed_nozzle: { diameter: "0.4", material: "stainless" },
        slots: [
          { feed: "ams", filament_identity: null, color: null, tag_uid: null },
          { feed: "ams", filament_identity: null, color: null, tag_uid: null },
          { feed: "ams", filament_identity: null, color: null, tag_uid: null },
          { feed: "ams", filament_identity: null, color: null, tag_uid: null },
          { feed: "direct", filament_identity: null, color: null, tag_uid: null },
        ],
      },
    ],
    bed: { identity: "Textured PEI Plate" },
    config_overrides: {},
    // Backend-computed view fields (PrinterInstanceView). `slots` is
    // unused by these pure helpers; `ams_units` is what initialDraft reads.
    slots: [],
    ams_units: 1,
    ...overrides,
  };
}

describe("initialDraft", () => {
  it("carries the backend AMS-units count into the draft", () => {
    const d = initialDraft(fixture());
    expect(d.amsUnits).toBe(1);
  });

  it("returns 0 when the printer is direct-feed only", () => {
    const d = initialDraft(fixture({ ams_units: 0 }));
    expect(d.amsUnits).toBe(0);
  });

  it("hydrates Bambu connection fields from the instance", () => {
    const conn: ConnectionInfo = {
      kind: "bambu",
      host: "192.168.1.42",
      access_code: "12345678",
    };
    const d = initialDraft(fixture({ connection: conn }));
    expect(d.host).toBe("192.168.1.42");
    expect(d.accessCode).toBe("12345678");
    expect(d.port).toBe(80);
  });

  it("hydrates U1 connection fields and defaults port to 80 when absent", () => {
    const conn: ConnectionInfo = {
      kind: "u1",
      host: "snappy.local",
      port: 8080,
    };
    const d = initialDraft(fixture({ connection: conn }));
    expect(d.host).toBe("snappy.local");
    expect(d.port).toBe(8080);
    expect(d.accessCode).toBe("");
  });

  it("defaults host/access_code/port to empty/80 when no connection is stored", () => {
    const d = initialDraft(fixture({ connection: null }));
    expect(d.host).toBe("");
    expect(d.accessCode).toBe("");
    expect(d.port).toBe(80);
  });
});

describe("computeChanged / computeSectionDirty", () => {
  const baseline = initialDraft(fixture());

  it("flags no fields when the draft equals the initial", () => {
    const changed = computeChanged(baseline, baseline);
    expect(changed).toEqual({
      displayName: false,
      amsUnits: false,
      host: false,
      accessCode: false,
      port: false,
    });
    const dirty = computeSectionDirty(changed, "bambu");
    expect(dirty.general).toBe(false);
    expect(dirty.connection).toBe(false);
  });

  it("rolls up displayName + amsUnits into the General section", () => {
    const renamed = { ...baseline, displayName: "Renamed" };
    const dirty = computeSectionDirty(
      computeChanged(baseline, renamed),
      "bambu",
    );
    expect(dirty.general).toBe(true);
    expect(dirty.connection).toBe(false);

    const amsBumped = { ...baseline, amsUnits: 0 };
    const dirty2 = computeSectionDirty(
      computeChanged(baseline, amsBumped),
      "bambu",
    );
    expect(dirty2.general).toBe(true);
  });

  it("rolls up host + accessCode into the Connection section for Bambu", () => {
    const hostEdit = { ...baseline, host: "new-ip" };
    expect(computeSectionDirty(computeChanged(baseline, hostEdit), "bambu")
      .connection).toBe(true);

    const codeEdit = { ...baseline, accessCode: "87654321" };
    expect(computeSectionDirty(computeChanged(baseline, codeEdit), "bambu")
      .connection).toBe(true);

    // Port edits never matter for Bambu — even when the value
    // diverges, the Bambu connection section stays clean.
    const portEdit = { ...baseline, port: 81 };
    expect(computeSectionDirty(computeChanged(baseline, portEdit), "bambu")
      .connection).toBe(false);
  });

  it("rolls up host + port into the Connection section for U1", () => {
    const portEdit = { ...baseline, port: 8080 };
    expect(computeSectionDirty(computeChanged(baseline, portEdit), "u1")
      .connection).toBe(true);

    // Access-code edits never matter for U1 — there's no access-code field.
    const codeEdit = { ...baseline, accessCode: "87654321" };
    expect(computeSectionDirty(computeChanged(baseline, codeEdit), "u1")
      .connection).toBe(false);
  });

  it("treats every connection edit as clean for an unknown driverKind", () => {
    const everything = {
      ...baseline,
      host: "x",
      accessCode: "y",
      port: 1,
    };
    expect(computeSectionDirty(computeChanged(baseline, everything), null)
      .connection).toBe(false);
  });
});

describe("validateBambuConnection", () => {
  it("accepts a non-empty host + 8-digit access code", () => {
    expect(validateBambuConnection("192.168.1.42", "12345678")).toBe(null);
  });
  it("rejects an empty / whitespace host", () => {
    expect(validateBambuConnection("", "12345678")?.field).toBe("host");
    expect(validateBambuConnection("   ", "12345678")?.field).toBe("host");
  });
  it("rejects an access code that isn't exactly 8 digits", () => {
    expect(validateBambuConnection("h", "1234567")?.field).toBe("accessCode");
    expect(validateBambuConnection("h", "123456789")?.field).toBe("accessCode");
    expect(validateBambuConnection("h", "1234567a")?.field).toBe("accessCode");
    expect(validateBambuConnection("h", "")?.field).toBe("accessCode");
  });
  it("trims surrounding whitespace before regex-checking the access code", () => {
    expect(validateBambuConnection("h", "  12345678  ")).toBe(null);
  });
});

describe("validateU1Connection", () => {
  it("accepts a non-empty host + port in 1..65535", () => {
    expect(validateU1Connection("snappy.local", 80)).toBe(null);
    expect(validateU1Connection("snappy.local", 1)).toBe(null);
    expect(validateU1Connection("snappy.local", 65535)).toBe(null);
  });
  it("rejects port 0 and out-of-range ports", () => {
    expect(validateU1Connection("h", 0)?.field).toBe("port");
    expect(validateU1Connection("h", 65536)?.field).toBe("port");
    expect(validateU1Connection("h", -1)?.field).toBe("port");
  });
  it("rejects non-integer ports", () => {
    expect(validateU1Connection("h", 80.5)?.field).toBe("port");
    expect(validateU1Connection("h", Number.NaN)?.field).toBe("port");
  });
  it("rejects empty host", () => {
    expect(validateU1Connection("", 80)?.field).toBe("host");
  });
});

describe("machine settings page order", () => {
  const opt = (key: string, category: string): OptionSummary => ({
    key,
    ty: "Float",
    label: key,
    category,
    group: null,
    default_value: { kind: "scalar", value: "0" },
    multiline: false,
    is_color: false,
    enum_values: [],
    tooltip: null,
    sidetext: null,
    mode: "simple",
    scope: { project: false, object: false, region: true },
    capability: null,
  });

  const arrange = (opts: OptionSummary[]) =>
    notesLast(orderGroupsOtherLast(categorize(opts, MACHINE_PAGE_ORDER))).map(
      (g) => g.id,
    );

  it("keeps Notes last, ahead of the build_unregular_pages pages", () => {
    // Options arrive display-order-sorted. `printer_notes` (Orca pos 497)
    // precedes the Motion ability / Multimaterial keys (515/525) because those
    // pages are coded in build_unregular_pages, after the Notes page — so naive
    // first-appearance would render Notes first. The curated order overrides it.
    const opts = [
      opt("printable_area", "Basic information"),
      opt("machine_start_gcode", "Machine G-code"),
      opt("printer_notes", "Notes"),
      opt("emit_machine_limits_to_gcode", "Motion ability"),
      opt("single_extruder_multi_material", "Multimaterial"),
    ];
    expect(arrange(opts)).toEqual([
      "Basic information",
      "Machine G-code",
      "Multimaterial",
      "Motion ability",
      "Notes",
    ]);
  });

  it("keeps any un-pinned section above Notes", () => {
    // notesLast guards the invariant directly: a section not in
    // MACHINE_PAGE_ORDER (e.g. a page a future Orca adds) would otherwise sort
    // after the pinned Notes. It must still land before it.
    const opts = [
      opt("printable_area", "Basic information"),
      opt("printer_notes", "Notes"),
      opt("some_new_key", "Future page"),
    ];
    expect(arrange(opts)).toEqual([
      "Basic information",
      "Future page",
      "Notes",
    ]);
  });
});
