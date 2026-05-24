// In-memory printer-credentials cache (PR-7a-7).
//
// Bambu LAN credentials (host + 8-char access code + serial) are
// session-scoped: lost on app reload, never written to the project
// .3mf. The user re-enters them per session via
// `PrinterCredentialsDialog`. Persistence is intentionally NOT
// implemented — see `memory/feedback_no_credentials_in_project_file.md`
// for the design directive.
//
// Keyed by `printer_identity` (the cascade-side identifier, e.g.
// `"bambu-a1-mini"`) so the same physical printer is reachable
// from any plate bound to that identity.

export interface BambuCredentials {
  host: string;
  access_code: string;
  serial: string | null;
}

import type { DriverId } from "./types";

const credentials: Map<string, BambuCredentials> = new Map();

/** Active driver registration per printer identity. Kept in
 * tandem with the credentials cache so the panel can detect a
 * "already connected" state on remount and skip re-registering
 * the same physical printer. */
const driverIds: Map<string, DriverId> = new Map();

export function getCredentials(
  printerIdentity: string,
): BambuCredentials | null {
  return credentials.get(printerIdentity) ?? null;
}

export function setCredentials(
  printerIdentity: string,
  creds: BambuCredentials,
): void {
  credentials.set(printerIdentity, creds);
}

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
