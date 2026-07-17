//! Snapmaker U1 camera wake: monitor mode over the driver's connection.
//!
//! The U1's camera daemon only writes fresh frames to
//! `/server/files/camera/monitor.jpg` while monitor mode is active, and
//! its capture watchdog hard-stops the mode ~361s after the last
//! `camera.start_monitor` — so a live view needs a start plus a periodic
//! re-send, and a `camera.stop_monitor` on teardown.
//!
//! The commands ride the printer's *status* connection: the camera holds
//! no link of its own. [`wake_task`] looks the driver up by instance and
//! sends through whatever [`ControlPlane`] its live session exposes —
//! mTLS MQTT for a paired printer, the open LAN WebSocket otherwise. The
//! method name (`camera.start_monitor`, dotted) and params are identical
//! on both transports, which is what makes one wake path serve the two.
//!
//! The frame fetch itself is the generic Moonraker JPEG poll
//! (`super::super::moonraker::webcam::poll_frame`) against
//! [`monitor_url`]; only the wake is Snapmaker-specific. The driving of
//! both from a `CameraSource` lives in `driver/camera.rs`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::core::driver::registry::DriverRegistry;

/// Re-send cadence, comfortably inside the daemon's ~361s watchdog.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
/// Retry cadence while the driver isn't connected yet (no control plane
/// to send through) or a send fails — short, so the camera wakes promptly
/// once the connection lands.
const RETRY_INTERVAL: Duration = Duration::from_secs(3);

/// The HTTP URL the monitor frame is polled from. Despite the daemon
/// returning a `url` field, the path Orca actually polls keeps the
/// `/server/` prefix (verified from a packet capture in the reference).
pub fn monitor_url(host: &str, port: u16) -> String {
    format!("http://{host}:{port}/server/files/camera/monitor.jpg")
}

/// Send one camera JSON-RPC through the instance's driver, if it is
/// registered and currently holds a live control plane. Returns whether
/// the send went out (not whether the daemon obeyed — both transports are
/// fire-and-forget here).
async fn send_camera(registry: &DriverRegistry, instance_id: &str, method: &str) -> bool {
    let Some(driver) = registry.find_by_instance(instance_id) else {
        return false;
    };
    let Some(control) = driver.read().await.control_plane() else {
        return false;
    };
    let params = json!({"domain": "lan", "interval": 1, "expect_pw": false});
    match control.send_jsonrpc(method, params).await {
        Ok(()) => true,
        Err(e) => {
            tracing::debug!(error = %e, "U1 camera {method} send failed");
            false
        }
    }
}

/// Keep monitor mode alive for the camera view's lifetime: send
/// `camera.start_monitor` now and re-send on [`HEARTBEAT_INTERVAL`],
/// retrying fast while the driver has no live connection. Runs until
/// aborted; pair it with [`release`] on teardown.
pub async fn wake_task(registry: Arc<DriverRegistry>, instance_id: String) {
    let mut woken = false;
    loop {
        let sent = send_camera(&registry, &instance_id, "camera.start_monitor").await;
        if sent && !woken {
            tracing::info!(instance_id, "U1 camera monitor started");
            woken = true;
        }
        tokio::time::sleep(if sent { HEARTBEAT_INTERVAL } else { RETRY_INTERVAL }).await;
    }
}

/// Release monitor mode: one best-effort `camera.stop_monitor`. The
/// daemon's watchdog is the backstop if the send doesn't land.
pub async fn release(registry: &DriverRegistry, instance_id: &str) {
    if send_camera(registry, instance_id, "camera.stop_monitor").await {
        tracing::debug!(instance_id, "U1 camera monitor stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_url_keeps_the_server_prefix() {
        assert_eq!(
            monitor_url("192.168.1.70", 80),
            "http://192.168.1.70:80/server/files/camera/monitor.jpg"
        );
    }

    /// No driver registered for the instance → nothing to send through;
    /// the wake loop keeps retrying but send_camera reports false.
    #[tokio::test]
    async fn send_camera_is_false_without_a_driver() {
        let registry = DriverRegistry::new();
        assert!(!send_camera(&registry, "ghost", "camera.start_monitor").await);
    }
}
