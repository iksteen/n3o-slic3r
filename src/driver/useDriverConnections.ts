// Reactive driver-registry manager.
//
// Watches every PrinterInstance.connection and reconciles the
// backend driver registry to match: register + connect when an
// instance gets a usable connection, disconnect + unregister when
// the connection is cleared or becomes incomplete, tear-down and
// rebuild when the saved values change. No UI affordance for
// connect/disconnect — the user-visible surface is just the
// settings modal's connection tab. State changes propagate from
// `setInstanceConnection` → `printer:instance_changed` event →
// `usePrinterInstances` refetch → this hook's key flips →
// reconciler runs.
//
// Module-scoped state for the live driver map: drivers outlive
// React renders and HMR refreshes. Listeners notify subscribed
// hooks when the map mutates so consumers re-render.

import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";
import { onEvents } from "../state/eventRouter";
import {
  driverConnect,
  driverDisconnect,
  driverRegister,
  driverUnregister,
} from "./invokes";
import type {
  ConnectionState,
  DriverConfig,
  DriverId,
  PrinterStatus,
  StatusUpdateEvent,
} from "./types";
import type {
  ConnectionInfo,
  PrinterInstance,
} from "../printer/printerInstance";
import { setInstanceAmsUnits } from "../printer/printerInstance";
import { isConnectionUsable } from "../printer/connectionValidation";
import { forgetDriver } from "./useDriverStatus";

// Re-export so existing importers (and the reconciler test) keep
// reaching it through this module. The definition lives in
// `connectionValidation` so the reconciler and the settings-modal
// save gate share one notion of "valid". See that module's header.
export { isConnectionUsable };

/** Wire-form config for the driver register call — matches the
 *  shape `driverRegister` expects (mirror of Rust `DriverConfig`,
 *  which uses an outer-tagged `kind` discriminator with PascalCase
 *  variants). */
export function configForConnection(conn: ConnectionInfo): DriverConfig {
  // The driver resolves the device serial itself at connect time
  // (Bambu peer-cert CN / U1 `/machine/system_info`), so the config
  // carries no serial.
  if (conn.kind === "bambu") {
    return {
      kind: "Bambu",
      data: {
        host: conn.host.trim(),
        access_code: conn.access_code.trim(),
      },
    };
  }
  return {
    kind: "U1",
    data: {
      host: conn.host.trim(),
      port: conn.port,
    },
  };
}

/** A stable signature of a `ConnectionInfo` — two connections with
 *  the same signature are interchangeable from the driver's POV.
 *  The reconciler uses this to detect "saved values changed" so it
 *  can tear down the old driver and rebuild with the new ones. The
 *  connection identity is `(host, access_code)` / `(host, port)` —
 *  the device serial isn't part of it (the driver probes the serial
 *  itself at connect time, so it never enters the persisted
 *  connection). */
export function connectionSignature(conn: ConnectionInfo | null): string {
  if (conn == null) return "none";
  if (conn.kind === "bambu") {
    return `bambu:${conn.host.trim()}|${conn.access_code.trim()}`;
  }
  return `u1:${conn.host.trim()}|${conn.port}`;
}

/** One row in the live driver table — the driver id + the signature
 *  of the connection it was registered with (so we can detect drift).
 *  Used by the test-seed helpers to stand up reconciler state. */
export interface DriverEntry {
  id: DriverId;
  signature: string;
}

// ── Module-scope reconciler state ────────────────────────────────
// Three maps, each with a distinct purpose:
//
//   * `ENTRIES` — per-(instance.id) state machine for the reconciler.
//     Tagged union: `in_flight` (mid-register), `live` (registered,
//     plus latest runtime status from the status bridge), `failed`
//     (last attempt threw). Absence = no driver registered.
//   * `DRIVER_TO_IDENTITY` — reverse map for the global status
//     listener, populated the moment `driverRegister` returns so
//     status events route correctly during the brief register →
//     connect window.
//   * `PENDING` — per-identity tail of the queue. Serializes
//     work for one identity; independent identities run in parallel.
//
// All keyed by PrinterInstance UUID (`instance.id`), NOT
// `vendor_profile_ref`, so two instances of the same printer model
// get distinct drivers + commands.
//
// React subscribes via `STATE_VERSION` + a SUBSCRIBERS set; every
// mutation bumps the version and notifies. `getSnapshotFor` caches
// the last summary by (instances reference, version) so consumers'
// prop identity stays stable when nothing changed (required by
// useSyncExternalStore and saves downstream React.memo'd children
// from churning).

/** Tagged-union state machine for one identity's reconciler entry.
 *  Absence in `ENTRIES` = no driver registered for this identity. */
export type ReconcilerEntry =
  | { kind: "in_flight" }
  | {
      kind: "live";
      id: DriverId;
      signature: string;
      runtime: ConnectionState | null;
    }
  | { kind: "failed"; reason: string };

const ENTRIES: Map<string, ReconcilerEntry> = new Map();
const DRIVER_TO_IDENTITY: Map<DriverId, string> = new Map();
const PENDING: Map<string, Promise<void>> = new Map();
const SUBSCRIBERS: Set<() => void> = new Set();
let STATE_VERSION = 0;

function bumpAndNotify(): void {
  STATE_VERSION++;
  for (const fn of SUBSCRIBERS) fn();
}

// ── Automatic AMS-count sync ─────────────────────────────────────
// The AMS-unit count chosen at add-printer time can diverge from the
// physical loadout (no AMS attached, a unit added/removed later), and
// a stale count routes prints at AMS slots that don't exist — a
// no-AMS P1 wedges in PREPARE on that. The live report is
// authoritative for the *topology*, so the unit count follows it
// automatically. Per-slot materials do NOT — those stay behind the
// explicit sync button (lossy round-trip, user-curated).
//
// `AMS_COUNT_SYNCED` holds the last count synced (or seeded from the
// instance itself at reconcile time) per instance id, so each
// reported count writes at most once: steady-state reports are
// silent, and a user who deliberately sets a different count in
// settings isn't fought until the printer actually reports a change.
const AMS_COUNT_SYNCED: Map<string, number> = new Map();

/** The AMS-unit count a status report carries, or `null` when it has
 *  nothing authoritative: U1 topology is fixed, and a Bambu report
 *  without an AMS state (`ams: null`) hasn't populated yet — never
 *  treat that as "0 units". */
export function reportedAmsUnits(status: PrinterStatus): number | null {
  if (status.extra.kind !== "Bambu") return null;
  const ams = status.extra.data.ams;
  return ams == null ? null : ams.units.length;
}

/** Sync the instance's AMS-unit count to a status report, once per
 *  distinct reported count. Fire-and-forget: the backend rebuilds the
 *  slot topology (preserving overlapping bindings) and emits
 *  `printer:instance_changed`, which refreshes the UI's instances. */
export function maybeSyncAmsCount(
  identity: string,
  status: PrinterStatus,
): void {
  const reported = reportedAmsUnits(status);
  if (reported == null || AMS_COUNT_SYNCED.get(identity) === reported) return;
  AMS_COUNT_SYNCED.set(identity, reported);
  setInstanceAmsUnits(identity, reported).catch((e: unknown) => {
    // Rejected counts (over ams_max, toolchanger) stay recorded so a
    // repeating report doesn't retry-spam; a genuine change in the
    // report clears the block by differing.
    console.warn(`[driver-auto] AMS-count sync failed for ${identity}`, e);
  });
}

/** One-shot install of the global `driver:status_update` handler via the
 *  router (which shares the one Tauri subscription with useDriverStatus). Lazy
 *  because the event bus isn't available during unit tests; the handler stays
 *  installed for the app's lifetime since drivers outlive the React tree. */
let globalStatusOff: (() => void) | null = null;
function ensureGlobalStatusListener(): void {
  if (globalStatusOff != null) return;
  globalStatusOff = onEvents<StatusUpdateEvent>(
    ["driver:status_update"],
    (e) => {
      // O(1) reverse-lookup. Populated at register-success time so
      // first-event-after-spawn routes correctly even before the entry
      // transitions to `live`.
      const identity = DRIVER_TO_IDENTITY.get(e.payload.driver_id);
      if (identity == null) return;
      maybeSyncAmsCount(identity, e.payload.status);
      const entry = ENTRIES.get(identity);
      if (entry == null || entry.kind !== "live") return;
      // In-place mutation of the runtime field is OK: the entry object
      // identity is not part of the snapshot cache key — the version bump
      // below invalidates it.
      ENTRIES.set(identity, {
        ...entry,
        runtime: e.payload.status.connection,
      });
      bumpAndNotify();
    },
  );
}

// Vite HMR teardown: when this module re-evaluates in dev, the
// old listener's closure still holds a reference to the prior
// module's LIVE/DRIVER_TO_IDENTITY maps; without the dispose hook
// it would stay subscribed forever and silently double every
// status event. Production builds skip this branch (no
// import.meta.hot).
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    // Detach only THIS module's handler — the router's shared
    // `driver:status_update` subscription stays up for useDriverStatus.
    globalStatusOff?.();
    globalStatusOff = null;
    ENTRIES.clear();
    DRIVER_TO_IDENTITY.clear();
    PENDING.clear();
    SUBSCRIBERS.clear();
    AMS_COUNT_SYNCED.clear();
  });
}

/** Picker-facing connection-status vocabulary. Mirrors the
 *  mockup's CONN_LABELS keys. */
export type ConnectionStatus =
  | "none"
  | "connecting"
  | "connected"
  | "failed";

/** Per-identity reconciler summary surfaced to React consumers.
 *  `driverId` is set whenever the reconciler successfully placed
 *  the driver in LIVE — i.e. `status === "connected"` — and null
 *  otherwise. `reason` only carries content for `status === "failed"`. */
export interface ConnectionSummary {
  status: ConnectionStatus;
  driverId: DriverId | null;
  reason: string | null;
}

function summaryFor(
  printerIdentity: string,
  conn: ConnectionInfo | null,
): ConnectionSummary {
  if (!isConnectionUsable(conn)) {
    return { status: "none", driverId: null, reason: null };
  }
  const entry = ENTRIES.get(printerIdentity);
  if (entry == null) {
    // Usable config saved, but no entry yet — either the brief
    // gap between dep-key flip and effect run, or right after
    // an unregister with no follow-up. Treat as connecting so
    // the dot doesn't blink grey-then-yellow.
    return { status: "connecting", driverId: null, reason: null };
  }
  switch (entry.kind) {
    case "in_flight":
      // Wins over any prior `failed` state — the new attempt is
      // optimistically "connecting" until it lands.
      return { status: "connecting", driverId: null, reason: null };
    case "failed":
      return { status: "failed", driverId: null, reason: entry.reason };
    case "live":
      // Signature drift: the saved connection has changed since
      // the reconciler last registered. A `replace` job is
      // queued — surface as "connecting" so the picker stops
      // pointing at the soon-to-be-killed driver. Callers that
      // dereference `driverId` get the old one until replace
      // lands; the status downgrade at least tells the user
      // something's mid-flight.
      if (entry.signature !== connectionSignature(conn)) {
        return { status: "connecting", driverId: entry.id, reason: null };
      }
      // Layer the live runtime state on top of the reconciler's
      // "registered" verdict so the picker reflects what the
      // network sees, not just what the last reconcile said.
      if (entry.runtime == null) {
        return { status: "connecting", driverId: entry.id, reason: null };
      }
      switch (entry.runtime.state) {
        case "Connected":
          return { status: "connected", driverId: entry.id, reason: null };
        case "Connecting":
          return { status: "connecting", driverId: entry.id, reason: null };
        case "Reconnecting":
          // Carry the driver's reconnect reason so the panel can show
          // why the link dropped while it retries (status stays
          // "connecting" — the picker dot doesn't distinguish).
          return {
            status: "connecting",
            driverId: entry.id,
            reason: entry.runtime.data.reason,
          };
        case "Disconnected":
          return {
            status: "failed",
            driverId: entry.id,
            reason: entry.runtime.data.reason,
          };
      }
  }
}

/** Cache for `snapshotFor`. Keyed by (instances reference,
 *  STATE_VERSION) so consumers receive a stable object reference
 *  when nothing relevant changed — required by useSyncExternalStore
 *  (otherwise React detects "changed" on every render and loops)
 *  and saves downstream React.memo'd children from refiring. */
let SNAPSHOT_CACHE: {
  instances: PrinterInstance[];
  version: number;
  result: Record<string, ConnectionSummary>;
} | null = null;

function snapshotFor(
  instances: PrinterInstance[],
): Record<string, ConnectionSummary> {
  if (
    SNAPSHOT_CACHE != null &&
    SNAPSHOT_CACHE.instances === instances &&
    SNAPSHOT_CACHE.version === STATE_VERSION
  ) {
    return SNAPSHOT_CACHE.result;
  }
  const result: Record<string, ConnectionSummary> = {};
  for (const inst of instances) {
    result[inst.id] = summaryFor(inst.id, inst.connection);
  }
  SNAPSHOT_CACHE = { instances, version: STATE_VERSION, result };
  return result;
}

/** Queue `work` behind any in-flight reconciliation for `identity`.
 *  Returns a promise that resolves once `work` runs. Independent
 *  identities never block each other. Failures don't propagate
 *  across the queue — each work item gets its own catch. */
function enqueueForIdentity(
  identity: string,
  work: () => Promise<void>,
): Promise<void> {
  const prior = PENDING.get(identity) ?? Promise.resolve();
  const next = prior.catch(() => {}).then(work);
  PENDING.set(identity, next);
  // Once this work resolves (success or failure), drop the slot
  // if we're still the tail — otherwise another reconcile already
  // queued behind us and is the new tail.
  void next.catch(() => {}).finally(() => {
    if (PENDING.get(identity) === next) PENDING.delete(identity);
  });
  return next;
}

/** Converge one identity's driver state to match its desired
 *  connection (null = no driver). Re-reads ENTRIES inside the
 *  queued critical section so duplicate plans converge to the
 *  right single effect; safe to call N times for the same identity. */
async function reconcileIdentity(
  identity: string,
  desiredConn: ConnectionInfo | null,
): Promise<void> {
  return enqueueForIdentity(identity, async () => {
    const existing = ENTRIES.get(identity);
    const liveEntry = existing?.kind === "live" ? existing : null;
    const wantSignature = desiredConn
      ? connectionSignature(desiredConn)
      : null;

    if (desiredConn == null) {
      if (liveEntry == null) {
        // Either absent or in a non-live state (in_flight /
        // failed). Drop any stale entry so the absence is
        // recorded.
        if (existing != null) {
          ENTRIES.delete(identity);
          bumpAndNotify();
        }
        return;
      }
      await unregisterDriver(identity, liveEntry.id);
      return;
    }
    if (liveEntry == null) {
      await registerDriver(identity, desiredConn);
      return;
    }
    if (liveEntry.signature !== wantSignature) {
      await replaceDriver(identity, liveEntry.id, desiredConn);
      return;
    }
    // existing matches desired — no-op.
  });
}

/** Register + connect a new driver for `identity`. Drives the
 *  state machine from absent/failed → in_flight → live (or
 *  → failed on register/connect error). Mirrors mutations into
 *  DRIVER_TO_IDENTITY so the global status listener can route
 *  events as soon as the driver id exists. */
async function registerDriver(
  identity: string,
  conn: ConnectionInfo,
): Promise<void> {
  ENTRIES.set(identity, { kind: "in_flight" });
  bumpAndNotify();
  let id: DriverId | null = null;
  try {
    id = await driverRegister(configForConnection(conn));
    // Populate the reverse map BEFORE driverConnect so any status
    // events that fire during the connect window route correctly.
    DRIVER_TO_IDENTITY.set(id, identity);
    try {
      await driverConnect(id);
    } catch (e) {
      // Connect failed — surface "failed" before the (slow)
      // unregister rollback so the picker can demote
      // immediately instead of sitting on "connecting" for the
      // rollback duration.
      console.warn(`[driver-auto] connect failed for ${identity}`, e);
      ENTRIES.set(identity, { kind: "failed", reason: String(e) });
      DRIVER_TO_IDENTITY.delete(id);
      bumpAndNotify();
      try {
        await driverUnregister(id);
      } catch (cleanupErr) {
        console.warn(
          `[driver-auto] register rollback failed for ${identity}`,
          cleanupErr,
        );
      }
      return;
    }
    ENTRIES.set(identity, {
      kind: "live",
      id,
      signature: connectionSignature(conn),
      runtime: null,
    });
    bumpAndNotify();
    // No explicit driverStatus(id) bootstrap call: the backend
    // `spawn_status_bridge` (core/driver/commands.rs) emits the
    // first PrinterStatus on spawn, and the global listener
    // routes it via DRIVER_TO_IDENTITY (populated above). The
    // picker sits in "connecting" until that event arrives —
    // usually a handful of ms.
  } catch (e) {
    console.warn(`[driver-auto] register failed for ${identity}`, e);
    ENTRIES.set(identity, { kind: "failed", reason: String(e) });
    if (id != null) DRIVER_TO_IDENTITY.delete(id);
    bumpAndNotify();
  }
}

/** Disconnect + unregister an existing driver for `identity`. */
async function unregisterDriver(
  identity: string,
  driverId: DriverId,
): Promise<void> {
  ENTRIES.delete(identity);
  DRIVER_TO_IDENTITY.delete(driverId);
  // Driver ids are monotonic and never reused, so drop the status
  // store's cached entry — nothing will read this id again.
  forgetDriver(driverId);
  bumpAndNotify();
  try {
    await driverDisconnect(driverId);
  } catch (e) {
    console.warn(`[driver-auto] disconnect failed for ${identity}`, e);
  }
  try {
    await driverUnregister(driverId);
  } catch (e) {
    console.warn(`[driver-auto] unregister failed for ${identity}`, e);
  }
}

/** Tear down the existing driver and register a new one with the
 *  fresh connection. Notifies subscribers between the teardown and
 *  the rebuild so the picker dot demotes to "connecting"
 *  immediately instead of sitting on stale "connected" through
 *  the (slow) MQTT disconnect. */
async function replaceDriver(
  identity: string,
  oldId: DriverId,
  newConn: ConnectionInfo,
): Promise<void> {
  ENTRIES.delete(identity);
  DRIVER_TO_IDENTITY.delete(oldId);
  forgetDriver(oldId);
  bumpAndNotify();
  try {
    await driverDisconnect(oldId);
  } catch (e) {
    console.warn(`[driver-auto] replace.disconnect failed for ${identity}`, e);
  }
  try {
    await driverUnregister(oldId);
  } catch (e) {
    console.warn(`[driver-auto] replace.unregister failed for ${identity}`, e);
  }
  await registerDriver(identity, newConn);
}

/** Reactive driver-registry manager. Mount once at the App level;
 *  pass the full instance list. Returns a `Record<instance.id,
 *  ConnectionSummary>` snapshot — the picker (dot indicator), the
 *  Devices view (per-printer monitor), and SendControls (driver id)
 *  all read off this.
 *
 *  Re-runs the reconciler whenever the connection signatures of any
 *  instance change. Connection-irrelevant edits (slot color, bed
 *  swap) don't refire because the dep key only hashes
 *  `instance.id` + the connection signature. */
export function useDriverConnections(
  instances: PrinterInstance[],
): Record<string, ConnectionSummary> {
  // useSyncExternalStore handles the module-scope subscription
  // correctly (SSR + concurrent-mode safe; no first-render-event
  // miss-window). snapshotFor memoizes by (instances, version) so
  // the result identity is stable when neither changed — required
  // by useSyncExternalStore to avoid render loops.
  const subscribe = useCallback((onStoreChange: () => void) => {
    SUBSCRIBERS.add(onStoreChange);
    return () => {
      SUBSCRIBERS.delete(onStoreChange);
    };
  }, []);
  const getSnapshot = useCallback(
    () => snapshotFor(instances),
    [instances],
  );
  const summaries = useSyncExternalStore(subscribe, getSnapshot);

  useEffect(() => {
    ensureGlobalStatusListener();
  }, []);

  // Stable key — only the connection-relevant subset, keyed by
  // instance UUID so two instances of the same printer model get
  // distinct dep slots.
  const key = useMemo(
    () =>
      instances
        .map((i) => `${i.id}|${connectionSignature(i.connection)}`)
        .sort()
        .join("\n"),
    [instances],
  );

  useEffect(() => {
    // Schedule a reconcile job per identity. Each call queues
    // behind any in-flight work for that identity (via PENDING)
    // and re-reads LIVE inside the queued critical section, so
    // duplicate/stale plans converge to the right single effect.
    // Independent identities run in parallel.
    const known = new Set<string>();
    for (const inst of instances) {
      known.add(inst.id);
      // Seed the AMS-count sync with the instance's own count so the
      // first status report only writes when it actually diverges.
      if (!AMS_COUNT_SYNCED.has(inst.id)) {
        AMS_COUNT_SYNCED.set(inst.id, inst.ams_units);
      }
      const desired = isConnectionUsable(inst.connection)
        ? inst.connection!
        : null;
      void reconcileIdentity(inst.id, desired);
    }
    // Identities present in ENTRIES but no longer in `instances`
    // (printer was deleted) get unregistered.
    for (const identity of ENTRIES.keys()) {
      if (!known.has(identity)) {
        AMS_COUNT_SYNCED.delete(identity);
        void reconcileIdentity(identity, null);
      }
    }
    // The `key` is the dep; `instances` is just read inside the
    // closure. ESLint can't see through the derivation, so we
    // disable the exhaustive-deps lint here on purpose.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  return summaries;
}

/** Test helper — wipe every reconciler-state map. Useful only for
 *  unit tests that want a clean starting point. Never call from
 *  production code. */
export function resetDriverConnectionsForTests(): void {
  ENTRIES.clear();
  DRIVER_TO_IDENTITY.clear();
  PENDING.clear();
  AMS_COUNT_SYNCED.clear();
  SNAPSHOT_CACHE = null;
  STATE_VERSION = 0;
}

/** Test helper — seed the AMS-count sync map (normally seeded from
 *  the instance list inside the reconcile effect). */
export function seedAmsCountForTests(identity: string, units: number): void {
  AMS_COUNT_SYNCED.set(identity, units);
}

/** Test helper — seed reconciler-state maps so the summary
 *  helpers can be exercised without driving the full reconciler.
 *  Adapter shape: existing tests use `live` / `inFlight` /
 *  `failed` / `runtimeStatus` partitions; this maps them onto the
 *  unified ReconcilerEntry. When the same identity appears in
 *  multiple partitions the precedence is in_flight > live >
 *  failed (matches the production state machine — in_flight wins
 *  because it represents an active attempt, even if a prior one
 *  failed; live wins over a stale failure record). */
export interface ReconcilerStateForTests {
  live?: Iterable<[string, DriverEntry]>;
  inFlight?: Iterable<string>;
  failed?: Iterable<[string, string]>;
  runtimeStatus?: Iterable<[string, ConnectionState]>;
}
export function seedReconcilerStateForTests(
  state: ReconcilerStateForTests,
): void {
  if (state.failed) {
    for (const [k, v] of state.failed) {
      ENTRIES.set(k, { kind: "failed", reason: v });
    }
  }
  if (state.live) {
    for (const [k, v] of state.live) {
      ENTRIES.set(k, {
        kind: "live",
        id: v.id,
        signature: v.signature,
        runtime: null,
      });
    }
  }
  if (state.runtimeStatus) {
    for (const [k, v] of state.runtimeStatus) {
      const e = ENTRIES.get(k);
      if (e?.kind === "live") {
        ENTRIES.set(k, { ...e, runtime: v });
      }
    }
  }
  if (state.inFlight) {
    for (const k of state.inFlight) {
      ENTRIES.set(k, { kind: "in_flight" });
    }
  }
  SNAPSHOT_CACHE = null;
}

/** Test helper — pure read of the summary for one identity.
 *  Snapshots the current module-state without running the
 *  reconciler. */
export function summaryForTests(
  printerIdentity: string,
  conn: ConnectionInfo | null,
): ConnectionSummary {
  return summaryFor(printerIdentity, conn);
}
