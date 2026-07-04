// Pure helpers + draft model for the per-printer settings modal
// (PrinterSettingsModal). Extracted so the modal file stays the
// orchestrator and these stay independently testable.

import {
  type ConnectionInfo,
  type PrinterInstance,
} from "./printerInstance";
import {
  validateConnectionInfo,
  type ConnectionFieldError,
} from "./connectionValidation";

/** Bambu printers need a LAN access code; U1 needs a Moonraker port.
 *  Default Moonraker port matches the existing PrinterCredentialsDialog. */
export const DEFAULT_U1_PORT = 80;

/** OrcaSlicer's machine-settings page order for the front pages. NOT derivable
 *  from the display-order scrape: Motion ability / Multimaterial are built in
 *  `TabPrinter::build_unregular_pages` (which runs *after* the Notes page in
 *  Tab.cpp) but `m_pages.insert`-ed to render *before* Notes — so a file-order
 *  scrape can't place them. Notes is not listed here — it's pinned last by
 *  `notesLast` regardless of which sections exist. Extruder pages are omitted
 *  (n3o extracts them into their own tabs). */
export const MACHINE_PAGE_ORDER = [
  "Basic information",
  "Machine G-code",
  "Multimaterial",
  "Motion ability",
] as const;

/** categorize() emits canonical categories (incl. the "Other" catch-all)
 *  before the scraped TabPrinter pages; push "Other"/"Others" last so the
 *  real pages lead in the printer-settings nav. */
export function orderGroupsOtherLast<T extends { id: string }>(groups: T[]): T[] {
  const isOther = (id: string) => id === "Other" || id === "Others";
  return [
    ...groups.filter((g) => !isOther(g.id)),
    ...groups.filter((g) => isOther(g.id)),
  ];
}

/** Motion ability rows OrcaSlicer hides entirely (its `toggle_line` calls in
 *  `TabPrinter::toggle_lines`), keyed on the printer's resolved `gcode_flavor`.
 *  These are firmware-specific: acceleration-travel isn't emitted by Marlin
 *  legacy / Klipper; junction deviation is Marlin-firmware only; input shaping
 *  is Marlin-firmware / RepRap-firmware only. Flavor strings are libslic3r's
 *  serialized enum keys ("marlin" = Marlin legacy, "marlin2" = Marlin firmware).
 *  `toggle_option` (grey-but-visible) is intentionally NOT mirrored — only the
 *  rows Orca removes from the page. */
const INPUT_SHAPING_KEYS = [
  "input_shaping_emit",
  "input_shaping_type",
  "input_shaping_freq_x",
  "input_shaping_freq_y",
  "input_shaping_damp_x",
  "input_shaping_damp_y",
];

export function firmwareHiddenKeys(
  gcodeFlavor: string | undefined,
): ReadonlySet<string> {
  const isMarlinLegacy = gcodeFlavor === "marlin";
  const isMarlinFirmware = gcodeFlavor === "marlin2";
  const isKlipper = gcodeFlavor === "klipper";
  const isReprapFirmware = gcodeFlavor === "reprapfirmware";
  const hidden = new Set<string>();
  if (isMarlinLegacy || isKlipper) hidden.add("machine_max_acceleration_travel");
  if (!isMarlinFirmware) hidden.add("machine_max_junction_deviation");
  if (!(isMarlinFirmware || isReprapFirmware)) {
    for (const k of INPUT_SHAPING_KEYS) hidden.add(k);
  }
  return hidden;
}

/** Collapse consecutive rows sharing a `.line` label into one block (Orca's
 *  multi-option lines — "Resonance Avoidance Speed" over its Min/Max pair).
 *  Rows with `line == null` each become their own single-row block. Only
 *  *consecutive* same-line rows group, matching the display-order layout. */
export function groupConsecutiveByLine<T extends { line: string | null }>(
  rows: readonly T[],
): { line: string | null; rows: T[] }[] {
  const out: { line: string | null; rows: T[] }[] = [];
  for (const row of rows) {
    const last = out[out.length - 1];
    if (row.line && last && last.line === row.line) last.rows.push(row);
    else out.push({ line: row.line, rows: [row] });
  }
  return out;
}

/** Notes is OrcaSlicer's terminal printer page. Pin it last unconditionally so
 *  no un-pinned section (a page not in MACHINE_PAGE_ORDER) can render below it —
 *  the invariant the curated MACHINE_PAGE_ORDER alone couldn't guarantee. */
export function notesLast<T extends { id: string }>(groups: T[]): T[] {
  const isNotes = (id: string) => id === "Notes";
  return [
    ...groups.filter((g) => !isNotes(g.id)),
    ...groups.filter((g) => isNotes(g.id)),
  ];
}

/** Pull the driver kind off the bound printer profile (authored in
 *  the printer's `model.toml`, carried through by the registry).
 *  Single source of truth — no inline string-prefix branches. */
export function driverKindFromProfile(
  profile: { driver_kind: "bambu" | "u1" | null } | null,
): "bambu" | "u1" | null {
  return profile?.driver_kind ?? null;
}

/** Draft shape — mirrors the editable fields. `connection` carries a
 *  superset that gets narrowed at save time per driver kind. */
export interface Draft {
  displayName: string;
  amsUnits: number;
  /** Bambu + U1 shared. */
  host: string;
  /** Bambu only. */
  accessCode: string;
  /** U1 only. */
  port: number;
}

/** Per-field dirty roll-up between an initial and current draft.
 *  Each flag mirrors a single editable surface in the modal; the
 *  view consumes these to mark `.changed` on the corresponding
 *  `.psm-field`. */
export interface DraftChanged {
  displayName: boolean;
  amsUnits: boolean;
  host: boolean;
  accessCode: boolean;
  port: boolean;
}

export function computeChanged(initial: Draft, draft: Draft): DraftChanged {
  return {
    displayName: draft.displayName !== initial.displayName,
    amsUnits: draft.amsUnits !== initial.amsUnits,
    host: draft.host !== initial.host,
    accessCode: draft.accessCode !== initial.accessCode,
    port: draft.port !== initial.port,
  };
}

/** Section-level roll-up of `DraftChanged`. Drives both the
 *  per-tab `psm-nav-dot` indicator and the top-level "is the modal
 *  dirty" check (which gates the Save button + discard overlay). */
export function computeSectionDirty(
  changed: DraftChanged,
  driverKind: "bambu" | "u1" | null,
): { general: boolean; connection: boolean } {
  const connection =
    driverKind === "bambu"
      ? changed.host || changed.accessCode
      : driverKind === "u1"
        ? changed.host || changed.port
        : false;
  return {
    general: changed.displayName || changed.amsUnits,
    connection,
  };
}

export function initialDraft(instance: PrinterInstance): Draft {
  const conn = instance.connection;
  let host = "";
  let accessCode = "";
  let port = DEFAULT_U1_PORT;
  if (conn?.kind === "bambu") {
    host = conn.host;
    accessCode = conn.access_code;
  } else if (conn?.kind === "u1") {
    host = conn.host;
    port = conn.port;
  }
  return {
    displayName: instance.display_name,
    amsUnits: instance.ams_units,
    host,
    accessCode,
    port,
  };
}

/** Build a `ConnectionInfo` from the draft for a given driver kind,
 *  or `null` for an unknown kind. The single place that knows the
 *  per-kind field layout + trimming — both `handleSave` (to build
 *  the patch) and `validateDraftConnection` (to validate) go through
 *  it, so adding a connection field is one edit, not four. */
export function draftToConnection(
  driverKind: "bambu" | "u1" | null,
  draft: Draft,
): ConnectionInfo | null {
  if (driverKind === "bambu") {
    return {
      kind: "bambu",
      host: draft.host.trim(),
      access_code: draft.accessCode.trim(),
    };
  }
  if (driverKind === "u1") {
    return {
      kind: "u1",
      host: draft.host.trim(),
      port: draft.port,
    };
  }
  return null;
}

export function validateDraftConnection(
  driverKind: "bambu" | "u1" | null,
  draft: Draft,
): ConnectionFieldError | null {
  const conn = draftToConnection(driverKind, draft);
  return conn == null ? null : validateConnectionInfo(conn);
}
