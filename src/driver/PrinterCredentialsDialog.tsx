// PR-7a-7 — modal asking for Bambu LAN credentials before the
// driver registers + connects.
//
// Three text inputs: host, 8-char access code (LCD-shown), serial
// (optional — the driver probes the peer cert CN if omitted).
// On submit: driver_register → driver_connect; on failure both
// error states surface inline + the driver is unregistered so the
// user can retry from a clean slate.
//
// Credentials are NOT persisted anywhere — they live in the
// in-memory cache (`credentialsCache.ts`) for the duration of the
// app session and are discarded on reload. See
// `memory/feedback_no_credentials_in_project_file.md` for the
// design directive.

import { useState } from "react";
import {
  driverConnect,
  driverRegister,
  driverUnregister,
} from "./invokes";
import { setCredentials, type BambuCredentials } from "./credentialsCache";
import type { DriverId } from "./types";

export interface PrinterCredentialsDialogProps {
  /** Cascade-side printer identity. Used as the credentials-cache
   * key so the same physical printer is reachable from any plate
   * bound to this identity. */
  printerIdentity: string;
  /** Pre-fill values shown on first open. Lets callers seed from
   * the cache if a previous session entered them. `null` fields
   * render empty. */
  initial?: Partial<BambuCredentials>;
  /** Called with the registered `DriverId` after `driver_connect`
   * resolves. The credentials are already in the cache by the
   * time this fires. */
  onConnected(id: DriverId): void;
  /** Dismiss without registering. */
  onCancel(): void;
}

/** Pure validator — extracted for tests. Returns the first error
 * encountered, or null if every field is acceptable. The access
 * code is always 8 numeric characters; host is non-empty; serial
 * is optional but if provided must be non-empty after trimming. */
export function validateCredentials(input: BambuCredentials): string | null {
  if (input.host.trim().length === 0) {
    return "Host is required";
  }
  if (!/^[0-9]{8}$/.test(input.access_code)) {
    return "Access code must be 8 digits (shown on the printer LCD)";
  }
  if (input.serial != null && input.serial.trim().length === 0) {
    // Treat an empty-but-not-null serial as "leave blank for probe".
    return null;
  }
  return null;
}

export function PrinterCredentialsDialog(
  props: PrinterCredentialsDialogProps,
): React.JSX.Element {
  const [host, setHost] = useState(props.initial?.host ?? "");
  const [accessCode, setAccessCode] = useState(
    props.initial?.access_code ?? "",
  );
  const [serial, setSerial] = useState(props.initial?.serial ?? "");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent): Promise<void> => {
    e.preventDefault();
    const trimmedSerial = serial.trim();
    const creds: BambuCredentials = {
      host: host.trim(),
      access_code: accessCode.trim(),
      serial: trimmedSerial.length > 0 ? trimmedSerial : null,
    };
    const validation = validateCredentials(creds);
    if (validation) {
      setError(validation);
      return;
    }
    setSubmitting(true);
    setError(null);

    let registeredId: DriverId | null = null;
    try {
      registeredId = await driverRegister({
        kind: "Bambu",
        data: {
          host: creds.host,
          access_code: creds.access_code,
          serial: creds.serial,
        },
      });
      await driverConnect(registeredId);
      // Only stash credentials AFTER connect resolves successfully
      // — failing creds shouldn't pollute the cache.
      setCredentials(props.printerIdentity, creds);
      props.onConnected(registeredId);
    } catch (e) {
      const message = String(e);
      setError(`Connect failed: ${message}`);
      // Roll back the registration so a retry doesn't accumulate
      // dead driver instances.
      if (registeredId != null) {
        try {
          await driverUnregister(registeredId);
        } catch (cleanupErr) {
          console.error(
            "[printer-credentials] rollback failed",
            cleanupErr,
          );
        }
      }
      setSubmitting(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
      role="dialog"
      aria-modal="true"
      aria-labelledby="printer-credentials-title"
    >
      <form
        onSubmit={handleSubmit}
        className="bg-surface text-text border border-border rounded-lg w-96 p-5 shadow-lg flex flex-col gap-3"
      >
        <h2
          id="printer-credentials-title"
          className="text-sm font-semibold"
        >
          Connect to {props.printerIdentity}
        </h2>
        <p className="text-xs text-text-muted">
          Enter the printer's LAN host and the 8-digit access code shown
          on its LCD under Settings → Network. Credentials stay in
          memory for this session only.
        </p>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-text-muted">Host (IP or .local name)</span>
          <input
            type="text"
            value={host}
            onChange={(e) => setHost(e.target.value)}
            placeholder="192.168.1.42"
            className="bg-surface-2 border border-border rounded px-2 py-1 text-sm font-mono"
            autoFocus
            disabled={submitting}
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-text-muted">Access code (8 digits)</span>
          <input
            type="text"
            inputMode="numeric"
            value={accessCode}
            onChange={(e) => setAccessCode(e.target.value)}
            placeholder="12345678"
            maxLength={8}
            className="bg-surface-2 border border-border rounded px-2 py-1 text-sm font-mono"
            disabled={submitting}
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-text-muted">
            Serial (optional — probed from cert if blank)
          </span>
          <input
            type="text"
            value={serial}
            onChange={(e) => setSerial(e.target.value)}
            placeholder="01S00A123400000"
            className="bg-surface-2 border border-border rounded px-2 py-1 text-sm font-mono"
            disabled={submitting}
          />
        </label>
        {error && (
          <div
            className="text-xs text-danger bg-danger/10 border border-danger/30 rounded px-2 py-1"
            role="alert"
          >
            {error}
          </div>
        )}
        <div className="flex gap-2 justify-end mt-1">
          <button
            type="button"
            onClick={props.onCancel}
            disabled={submitting}
            className="px-3 py-1 text-xs border border-border rounded hover:bg-surface-2"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={submitting}
            className="px-3 py-1 text-xs bg-accent text-white rounded hover:opacity-90 disabled:opacity-50"
          >
            {submitting ? "Connecting…" : "Connect"}
          </button>
        </div>
      </form>
    </div>
  );
}
