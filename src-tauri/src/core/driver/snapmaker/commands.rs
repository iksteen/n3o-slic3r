//! Tauri commands for the Snapmaker U1 LAN camera pairing.
//!
//! Pairing yields the per-printer mTLS material the camera wake needs
//! ([`super::snap_token`]). The keypair stays server-side — these commands
//! never return it; the frontend only learns "paired ✓ (serial …)".

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::pairing;
use super::snap_token::{self, SnapToken};

/// Fired when an instance's pairing state changes (paired or unpaired), so
/// live consumers — notably the Devices camera panel — re-evaluate without
/// a remount. Payload is the instance id.
const PAIRING_CHANGED_EVENT: &str = "u1:pairing_changed";

/// How long we wait for the user to tap Approve on the printer.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);

/// Pairing status for an instance, surfaced to the Connection settings UI.
/// Never carries key material — just whether we're paired and, if so, the
/// printer serial to display.
#[derive(Debug, Serialize)]
pub struct PairingStatus {
    pub paired: bool,
    pub serial: Option<String>,
}

/// Run the U1 pairing dance for `instance_id` against `host`, persisting
/// the resulting token. Reuses the prior pairing's stable `clientid` (if
/// any) so a re-pair against the same printer skips the on-screen approval.
///
/// This blocks until the user taps Approve on the printer (or the
/// ~60s timeout elapses) — the frontend shows a "tap Approve" prompt for
/// the duration. Returns the paired serial.
#[tracing::instrument(skip_all, fields(instance_id))]
#[tauri::command]
pub async fn u1_pair(
    app: AppHandle,
    instance_id: String,
    host: String,
) -> Result<PairingStatus, String> {
    let host = host.trim().to_owned();
    if host.is_empty() {
        return Err("printer host is empty".to_owned());
    }
    // Reuse the existing clientid so the printer recognizes us without a
    // fresh approval tap; otherwise mint a new one.
    let clientid = snap_token::load(&instance_id)
        .map(|t| t.clientid)
        .unwrap_or_else(pairing::fresh_clientid);

    // DriverError crosses the IPC boundary as its Display string, like every
    // other driver command, so the frontend shows the message.
    let token: SnapToken = pairing::pair(&host, &clientid, APPROVAL_TIMEOUT)
        .await
        .map_err(|e| e.to_string())?;
    let serial = token.sn.clone();
    snap_token::save(&instance_id, &token)
        .map_err(|e| format!("persist pairing token: {e}"))?;
    tracing::info!(serial = %serial, "U1 paired");
    let _ = app.emit(PAIRING_CHANGED_EVENT, &instance_id);

    Ok(PairingStatus {
        paired: true,
        serial: Some(serial),
    })
}

/// Whether `instance_id` is paired (and the serial, for display).
#[tauri::command]
pub fn u1_pairing_status(instance_id: String) -> PairingStatus {
    match snap_token::load(&instance_id) {
        Some(token) => PairingStatus {
            paired: true,
            serial: Some(token.sn),
        },
        None => PairingStatus {
            paired: false,
            serial: None,
        },
    }
}

/// Forget `instance_id`'s pairing. Idempotent.
#[tauri::command]
pub fn u1_unpair(app: AppHandle, instance_id: String) -> Result<(), String> {
    snap_token::delete(&instance_id).map_err(|e| format!("delete pairing token: {e}"))?;
    let _ = app.emit(PAIRING_CHANGED_EVENT, &instance_id);
    Ok(())
}
