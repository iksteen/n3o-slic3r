// Connection validity — the single source of truth shared by the
// settings-modal save gate and the driver reconciler.
//
// Before this module the two disagreed: the reconciler's
// `isConnectionUsable` only checked "host + access-code non-empty"
// while the modal's validators required an 8-digit Bambu code / a
// port in 1..65535. A hand-edited or partially-migrated connection
// could be "usable" to the reconciler (it would attempt to connect)
// yet "invalid" to the form — the picker dot and the modal said
// different things. Both now derive from `validateConnectionInfo`,
// so "valid" means one thing everywhere.

import type { ConnectionInfo } from "./printerInstance";

export interface ConnectionFieldError {
  field: "host" | "accessCode" | "port";
  message: string;
}

export function validateBambuConnection(
  host: string,
  accessCode: string,
): ConnectionFieldError | null {
  if (host.trim().length === 0) {
    return { field: "host", message: "Host is required" };
  }
  if (!/^[0-9a-fA-F]{8}$/.test(accessCode.trim())) {
    return {
      field: "accessCode",
      message:
        "Access code must be 8 characters, 0-9 or a-f (shown on the printer LCD)",
    };
  }
  return null;
}

/** Shared by the U1 and generic Moonraker kinds — both are a
 *  Moonraker endpoint reached by host + port. */
export function validateMoonrakerConnection(
  host: string,
  port: number,
): ConnectionFieldError | null {
  if (host.trim().length === 0) {
    return { field: "host", message: "Host is required" };
  }
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    return { field: "port", message: "Port must be between 1 and 65535" };
  }
  return null;
}

/** Validate a stored `ConnectionInfo`. Returns the first field
 *  problem, or `null` when the connection is complete and valid.
 *  `null` input (no connection saved) is reported as a missing-host
 *  error so callers can surface "configure a connection". */
export function validateConnectionInfo(
  conn: ConnectionInfo | null,
): ConnectionFieldError | null {
  if (conn == null) {
    return { field: "host", message: "Connection is required" };
  }
  if (conn.kind === "bambu") {
    return validateBambuConnection(conn.host, conn.access_code);
  }
  return validateMoonrakerConnection(conn.host, conn.port);
}

/** A connection the reconciler will attempt to register + connect.
 *  Defined as "passes the same validation the save form enforces",
 *  so the picker dot (driven by the reconciler) and the modal never
 *  disagree about what counts as valid. */
export function isConnectionUsable(conn: ConnectionInfo | null): boolean {
  return conn != null && validateConnectionInfo(conn) == null;
}
