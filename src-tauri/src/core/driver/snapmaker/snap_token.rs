//! Persisted Snapmaker U1 LAN pairing material.
//!
//! Pairing ([`super::pairing`]) hands us per-printer mutual-TLS material:
//! the printer's CA, our client cert + private key, the stable client
//! identifier the printer keys its auth DB on, and the serial number we
//! must use as the topic prefix once we reconnect over mTLS. The camera
//! wake ([`super::camera`]) is the consumer.
//!
//! These are **credentials**, so they live server-side only — one JSON
//! file per instance at `<printers_root>/<id>.snap.json`, next to the
//! instance `.toml`. They never enter `ConnectionInfo`, the wire
//! `DriverConfig`, or a shared `.3mf`; the frontend only ever learns
//! "paired ✓ (serial …)". The private key is redacted from `Debug`.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::printer::instance_storage;

/// Per-printer pairing material. `#[serde]`-persisted; the private key is
/// redacted from `Debug` so it never lands in a log line.
#[derive(Clone, Serialize, Deserialize)]
pub struct SnapToken {
    /// Host the user paired against (the Moonraker LAN address).
    pub host: String,
    /// Printer serial number from the auth response. The topic prefix for
    /// every per-device mTLS request (`<sn>/request`).
    pub sn: String,
    /// Stable client identifier presented to the printer's auth manager.
    /// Reused across re-pairs so the user need not re-tap Approve.
    pub clientid: String,
    /// mTLS MQTT port the printer told us to reconnect on (8883 observed).
    pub mqtt_port: u16,
    /// Printer-issued CA (PEM) — the trust anchor for the mTLS session.
    pub ca: String,
    /// Our client certificate (PEM).
    pub cert: String,
    /// Our client private key (PEM; PKCS#1 as shipped). Secret.
    pub key: String,
}

impl fmt::Debug for SnapToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapToken")
            .field("host", &self.host)
            .field("sn", &self.sn)
            .field("clientid", &self.clientid)
            .field("mqtt_port", &self.mqtt_port)
            .field("ca", &"<pem>")
            .field("cert", &"<pem>")
            .field("key", &"<redacted>")
            .finish()
    }
}

/// The token file for an instance, given the printers root.
fn token_path(root: &Path, instance_id: &str) -> PathBuf {
    root.join(format!("{instance_id}.snap.json"))
}

/// Load the pairing token for `instance_id`, or `None` if the instance
/// isn't paired (or no printers root is configured — e.g. in unit tests
/// that never call `init_root`).
pub fn load(instance_id: &str) -> Option<SnapToken> {
    let root = instance_storage::root()?;
    load_from(root, instance_id)
}

/// Persist the pairing token for `instance_id`, replacing any prior one.
pub fn save(instance_id: &str, token: &SnapToken) -> std::io::Result<()> {
    let root = instance_storage::root().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no printers root configured; cannot persist pairing token",
        )
    })?;
    save_to(root, instance_id, token)
}

/// Remove the pairing token for `instance_id`. Idempotent.
pub fn delete(instance_id: &str) -> std::io::Result<()> {
    let Some(root) = instance_storage::root() else {
        return Ok(());
    };
    delete_from(root, instance_id)
}

// ── Root-explicit forms (unit-testable without the global OnceLock) ──

pub fn load_from(root: &Path, instance_id: &str) -> Option<SnapToken> {
    let path = token_path(root, instance_id);
    let text = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str(&text) {
        Ok(token) => Some(token),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "malformed snap token; ignoring");
            None
        }
    }
}

pub fn save_to(root: &Path, instance_id: &str, token: &SnapToken) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let path = token_path(root, instance_id);
    let body = serde_json::to_vec_pretty(token)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    // Atomic: temp sibling then rename, so a crash mid-write never leaves a
    // half-written credential file.
    crate::core::paths::atomic_write(&path, &body)
}

pub fn delete_from(root: &Path, instance_id: &str) -> std::io::Result<()> {
    let path = token_path(root, instance_id);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(sn: &str) -> SnapToken {
        SnapToken {
            host: "192.168.1.70".to_owned(),
            sn: sn.to_owned(),
            clientid: "n3o-abc".to_owned(),
            mqtt_port: 8883,
            ca: "-----BEGIN CERTIFICATE-----\nCA\n-----END CERTIFICATE-----\n".to_owned(),
            cert: "-----BEGIN CERTIFICATE-----\nCERT\n-----END CERTIFICATE-----\n".to_owned(),
            key: "-----BEGIN RSA PRIVATE KEY-----\nSECRETKEYMATERIAL\n-----END RSA PRIVATE KEY-----\n"
                .to_owned(),
        }
    }

    #[test]
    fn debug_redacts_the_private_key() {
        let debug = format!("{:?}", sample("SN1"));
        assert!(!debug.contains("SECRETKEYMATERIAL"));
        assert!(debug.contains("redacted"));
        assert!(debug.contains("SN1"), "non-secret fields still shown");
    }

    #[test]
    fn save_load_round_trips_and_delete_clears() {
        let dir = std::env::temp_dir().join(format!("n3o-snap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        assert!(load_from(&dir, "printer-a").is_none(), "unpaired → None");

        save_to(&dir, "printer-a", &sample("SN-A")).unwrap();
        save_to(&dir, "printer-b", &sample("SN-B")).unwrap();

        let a = load_from(&dir, "printer-a").expect("paired");
        assert_eq!(a.sn, "SN-A");
        assert_eq!(a.mqtt_port, 8883);
        // Independent per instance.
        assert_eq!(load_from(&dir, "printer-b").unwrap().sn, "SN-B");

        // Re-save replaces.
        save_to(&dir, "printer-a", &sample("SN-A-NEW")).unwrap();
        assert_eq!(load_from(&dir, "printer-a").unwrap().sn, "SN-A-NEW");

        delete_from(&dir, "printer-a").unwrap();
        assert!(load_from(&dir, "printer-a").is_none());
        // Delete of an unpaired instance is a no-op.
        delete_from(&dir, "printer-a").unwrap();
        // Sibling untouched.
        assert!(load_from(&dir, "printer-b").is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_temp_file_left_behind_after_save() {
        let dir = std::env::temp_dir().join(format!("n3o-snap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        save_to(&dir, "p", &sample("SN")).unwrap();
        let leftover = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.path().extension().and_then(|s| s.to_str()) == Some("tmp"));
        assert!(!leftover, "temp file should be renamed away");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
