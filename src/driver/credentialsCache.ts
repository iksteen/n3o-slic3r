// In-memory printer-credentials cache (PR-7a-7, extended in PR-7b-7).
//
// LAN credentials (Bambu host + 8-char access code + serial, or
// Snapmaker U1 host + port + optional serial) are session-scoped:
// lost on app reload, never written to the project .3mf. The user
// re-enters them per session via `PrinterCredentialsDialog`.
// Persistence is intentionally NOT implemented — see
// `memory/feedback_no_credentials_in_project_file.md`.
//
// Keyed by `printer_identity` (the cascade-side identifier, e.g.
// `"bambu-lab-a1-mini"` or `"snapmaker-u1"`) so the same physical
// printer is reachable from any plate bound to that identity.

import type { DriverId } from "./types";

export interface BambuCredentials {
  host: string;
  access_code: string;
  serial: string | null;
}

export interface U1Credentials {
  host: string;
  port: number;
  serial: string | null;
}

/** Discriminated union — store either Bambu or U1 creds per key.
 *  Reads return `null` rather than the wrong variant when the cache
 *  hit doesn't match the requested kind.
 */
type Credentials =
  | { kind: "Bambu"; data: BambuCredentials }
  | { kind: "U1"; data: U1Credentials };

const credentials: Map<string, Credentials> = new Map();

/** Active driver registration per printer identity. Kept in
 * tandem with the credentials cache so the panel can detect a
 * "already connected" state on remount and skip re-registering
 * the same physical printer. */
const driverIds: Map<string, DriverId> = new Map();

export function getBambuCredentials(
  printerIdentity: string,
): BambuCredentials | null {
  const entry = credentials.get(printerIdentity);
  return entry?.kind === "Bambu" ? entry.data : null;
}

export function setBambuCredentials(
  printerIdentity: string,
  creds: BambuCredentials,
): void {
  credentials.set(printerIdentity, { kind: "Bambu", data: creds });
}

export function getU1Credentials(
  printerIdentity: string,
): U1Credentials | null {
  const entry = credentials.get(printerIdentity);
  return entry?.kind === "U1" ? entry.data : null;
}

export function setU1Credentials(
  printerIdentity: string,
  creds: U1Credentials,
): void {
  credentials.set(printerIdentity, { kind: "U1", data: creds });
}

// Legacy aliases kept for callers that pre-date PR-7b-7's
// kind-aware split. New code should call the per-variant
// helpers above.
export const getCredentials = getBambuCredentials;
export const setCredentials = setBambuCredentials;

export function clearCredentials(printerIdentity: string): void {
  credentials.delete(printerIdentity);
}

export function getDriverId(printerIdentity: string): DriverId | null {
  return driverIds.get(printerIdentity) ?? null;
}

export function setDriverId(
  printerIdentity: string,
  id: DriverId,
): void {
  driverIds.set(printerIdentity, id);
}

export function clearDriverId(printerIdentity: string): void {
  driverIds.delete(printerIdentity);
}

/** Test helper — drops every cached entry. */
export function resetCredentialsCacheForTests(): void {
  credentials.clear();
  driverIds.clear();
}
