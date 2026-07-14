import { useEffect, useRef, useState } from "react";
import { configForConnection } from "../driver/useDriverConnections";
import {
  driverTestConnection,
  u1Pair,
  u1PairingStatus,
  u1Unpair,
  type PairingStatus,
} from "../driver/invokes";
import type { ConnectionFieldError } from "./connectionValidation";
import {
  draftToConnection,
  validateDraftConnection,
  type ConnectionDriverKind,
  type Draft,
} from "./printerSettingsHelpers";

/** U1 camera pairing control. Pairing obtains the printer's mTLS camera
 *  credentials via an on-screen Approve tap; the keypair is stored
 *  server-side (never here), so this only ever shows paired/unpaired + the
 *  serial. Lives in the U1 Connection tab, beside Test connection. */
function U1PairingControl({
  instanceId,
  host,
}: {
  instanceId: string;
  /** Current draft IP — pairing dials this. */
  host: string;
}): React.JSX.Element {
  const [status, setStatus] = useState<PairingStatus | null>(null);
  const [pairing, setPairing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Load the persisted pairing state for this instance.
  useEffect(() => {
    let alive = true;
    void u1PairingStatus(instanceId)
      .then((s) => {
        if (alive) setStatus(s);
      })
      .catch(() => {
        /* status stays null → treated as unpaired */
      });
    return () => {
      alive = false;
    };
  }, [instanceId]);

  const hostReady = host.trim().length > 0;
  const paired = status?.paired ?? false;

  const runPair = async (): Promise<void> => {
    setPairing(true);
    setError(null);
    try {
      setStatus(await u1Pair(instanceId, host.trim()));
    } catch (e) {
      setError(String(e));
    } finally {
      setPairing(false);
    }
  };

  const runUnpair = async (): Promise<void> => {
    setError(null);
    try {
      await u1Unpair(instanceId);
      setStatus({ paired: false, serial: null });
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="psm-field">
      <label>Camera pairing</label>
      <div className="psm-conn-note" role="note">
        <svg
          className="psm-conn-note-ico"
          width="15"
          height="15"
          viewBox="0 0 16 16"
          fill="none"
          aria-hidden="true"
        >
          <path
            d="M8 1.5l5.5 2.4v3.2c0 3.2-2.3 6-5.5 7-3.2-1-5.5-3.8-5.5-7V3.9L8 1.5z"
            stroke="currentColor"
            strokeWidth="1.2"
            strokeLinejoin="round"
          />
          <path
            d="M8 6v3.2M8 11.2h.01"
            stroke="currentColor"
            strokeWidth="1.3"
            strokeLinecap="round"
          />
        </svg>
        <div className="psm-conn-note-body">
          <strong>Pairing needs LAN mode — but only while pairing.</strong>{" "}
          <span>
            Enable <em>LAN Mode</em> on the printer, pair, then tap{" "}
            <em>Approve</em> when it prompts. Once paired you can reconnect
            the printer to the cloud — the pairing stays active.
          </span>
        </div>
      </div>
      <div className="psm-conn-test">
        <button
          type="button"
          className="apm-btn"
          onClick={() => void runPair()}
          disabled={!hostReady || pairing}
          title="Pair with the printer to enable the live camera. You'll tap Approve on the printer screen."
        >
          {pairing
            ? "Pairing… tap Approve on the printer"
            : paired
              ? "Re-pair"
              : "Pair with printer"}
        </button>
        {paired && status?.serial && (
          <span className="apm-name-hint psm-conn-test-ok">
            ✓ Paired ({status.serial})
          </span>
        )}
        {paired && !pairing && (
          <button
            type="button"
            className="psm-delete-trigger"
            onClick={() => void runUnpair()}
            title="Forget this pairing"
          >
            Unpair
          </button>
        )}
      </div>
      {error ? (
        <div className="apm-name-hint error">{error}</div>
      ) : (
        <div className="apm-name-hint">
          {paired
            ? "The live camera in Devices is enabled, and stays active even after the printer returns to cloud mode. Re-pair if you replaced the printer or cleared its pairing."
            : "Enables the live camera in Devices."}
        </div>
      )}
    </div>
  );
}

export function ConnectionSection({
  driverKind,
  instanceId,
  profileLabel,
  draft,
  setDraft,
  changed,
  fieldError,
  canForget,
  onForget,
  onEdit,
}: {
  driverKind: ConnectionDriverKind;
  instanceId: string;
  profileLabel: string;
  draft: Draft;
  setDraft: React.Dispatch<React.SetStateAction<Draft>>;
  changed: { host: boolean; accessCode: boolean; port: boolean };
  fieldError: ConnectionFieldError | null;
  /** True when the instance has a saved connection AND the
   *  user hasn't already clicked Forget this session — drives
   *  the visibility of the Forget button. */
  canForget: boolean;
  onForget: () => void;
  /** Called whenever the user edits a connection field. Cancels a
   *  pending "Forget connection" so re-entering credentials saves
   *  them instead of being overridden by the clear intent. */
  onEdit: () => void;
}): React.JSX.Element {
  const hostError = fieldError?.field === "host" ? fieldError.message : null;
  const portError = fieldError?.field === "port" ? fieldError.message : null;
  const codeError =
    fieldError?.field === "accessCode" ? fieldError.message : null;

  // "Test connection" — spins up a transient backend driver against
  // the current draft and reports the verdict inline. Independent of
  // Save: it never persists. Disabled while the draft is invalid or a
  // test is in flight.
  const [test, setTest] = useState<
    | { kind: "idle" }
    | { kind: "testing" }
    | { kind: "ok" }
    | { kind: "error"; message: string }
  >({ kind: "idle" });
  // Monotonic id identifying the in-flight test. Bumped both when a
  // connection field changes and when a new test starts, so a test
  // that resolves AFTER the user has edited a field is discarded
  // rather than painting a stale verdict against the new draft.
  const testRunRef = useRef(0);
  // A prior verdict is stale the moment any connection field changes;
  // bumping the run id also invalidates any in-flight test.
  useEffect(() => {
    testRunRef.current += 1;
    setTest({ kind: "idle" });
  }, [draft.host, draft.port, draft.accessCode]);
  // Gate the button on the DRAFT's own validity (via the shared
  // helper) rather than the parent's `fieldError`, which is suppressed
  // during Forget mode — so we never test a blanked/half-entered
  // connection.
  const testReady = validateDraftConnection(driverKind, draft) == null;
  const runTest = async (): Promise<void> => {
    const conn = draftToConnection(driverKind, draft);
    if (conn == null) return;
    const runId = (testRunRef.current += 1);
    setTest({ kind: "testing" });
    try {
      await driverTestConnection(configForConnection(conn));
      if (testRunRef.current === runId) setTest({ kind: "ok" });
    } catch (e) {
      if (testRunRef.current === runId) {
        setTest({ kind: "error", message: String(e) });
      }
    }
  };

  return (
    <div className="psm-section">
      <div className={`psm-field${changed.host ? " changed" : ""}`}>
        <label htmlFor="psm-conn-ip">IP address</label>
        <div className={`apm-name-input${hostError ? " error" : ""}`}>
          <input
            id="psm-conn-ip"
            value={draft.host}
            onChange={(e) => {
              onEdit();
              setDraft((d) => ({ ...d, host: e.target.value }));
            }}
            placeholder="e.g. 192.168.1.42"
            inputMode="decimal"
            autoComplete="off"
            spellCheck={false}
          />
        </div>
        {hostError ? (
          <div className="apm-name-hint error">{hostError}</div>
        ) : (
          <div className="apm-name-hint">
            The local network address of your {profileLabel}. Find it on the
            printer&rsquo;s screen under network settings.
          </div>
        )}
      </div>

      {(driverKind === "u1" || driverKind === "moonraker") && (
        <div className={`psm-field${changed.port ? " changed" : ""}`}>
          <label htmlFor="psm-conn-port">Port</label>
          <div className={`apm-name-input${portError ? " error" : ""}`}>
            <input
              id="psm-conn-port"
              value={draft.port === 0 ? "" : String(draft.port)}
              onChange={(e) => {
                onEdit();
                const digits = e.target.value.replace(/[^0-9]/g, "");
                const parsed = digits === "" ? 0 : Math.min(65535, Number(digits));
                setDraft((d) => ({ ...d, port: parsed }));
              }}
              placeholder="80"
              inputMode="numeric"
              autoComplete="off"
              spellCheck={false}
            />
          </div>
          {portError ? (
            <div className="apm-name-hint error">{portError}</div>
          ) : (
            <div className="apm-name-hint">
              The port n3o connects on. Defaults to 80; change it only if
              you&rsquo;ve remapped the printer&rsquo;s HTTP interface.
            </div>
          )}
        </div>
      )}

      {driverKind === "u1" && (
        <U1PairingControl instanceId={instanceId} host={draft.host} />
      )}

      {driverKind === "bambu" && (
        <div className={`psm-field${changed.accessCode ? " changed" : ""}`}>
          <label htmlFor="psm-conn-code">Access code</label>
          <div className={`apm-name-input${codeError ? " error" : ""}`}>
            <input
              id="psm-conn-code"
              value={draft.accessCode}
              onChange={(e) => {
                onEdit();
                setDraft((d) => ({ ...d, accessCode: e.target.value }));
              }}
              placeholder="8-character code"
              autoComplete="off"
              spellCheck={false}
            />
          </div>
          {codeError ? (
            <div className="apm-name-hint error">{codeError}</div>
          ) : (
            <div className="apm-name-hint">
              Shown on the printer under the LAN-only access settings. Used to
              authenticate this connection.
            </div>
          )}
        </div>
      )}

      {driverKind === "bambu" && (
        <div className="psm-conn-note" role="note">
          <svg
            className="psm-conn-note-ico"
            width="15"
            height="15"
            viewBox="0 0 16 16"
            fill="none"
            aria-hidden="true"
          >
            <path
              d="M8 1.5l5.5 2.4v3.2c0 3.2-2.3 6-5.5 7-3.2-1-5.5-3.8-5.5-7V3.9L8 1.5z"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinejoin="round"
            />
            <path
              d="M8 6v3.2M8 11.2h.01"
              stroke="currentColor"
              strokeWidth="1.3"
              strokeLinecap="round"
            />
          </svg>
          <div className="psm-conn-note-body">
            <strong>
              Requires LAN-only mode with Developer Mode enabled.
            </strong>{" "}
            <span>
              On the printer, turn on <em>LAN Only Mode</em> and{" "}
              <em>Developer Mode</em> before connecting — n3o can&rsquo;t reach
              it over the cloud.
            </span>
          </div>
        </div>
      )}

      <div className="psm-conn-test">
        <button
          type="button"
          className="apm-btn"
          onClick={() => void runTest()}
          disabled={!testReady || test.kind === "testing"}
          title="Try connecting with these settings without saving"
        >
          {test.kind === "testing" ? "Testing…" : "Test connection"}
        </button>
        {test.kind === "ok" && (
          <span className="apm-name-hint psm-conn-test-ok">✓ Connected</span>
        )}
        {test.kind === "error" && (
          <span className="apm-name-hint error" title={test.message}>
            ✗ {test.message}
          </span>
        )}
      </div>

      {canForget && (
        <button
          type="button"
          className="psm-delete-trigger"
          onClick={onForget}
          title="Clear the saved connection. The printer stays in your library; you can re-enter credentials any time."
        >
          Forget connection
        </button>
      )}
    </div>
  );
}
