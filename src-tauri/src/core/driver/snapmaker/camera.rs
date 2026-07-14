//! Snapmaker U1 camera plumbing: the mTLS "monitor mode" wake.
//!
//! The U1's camera daemon only writes fresh frames to
//! `/server/files/camera/monitor.jpg` while monitor mode is active;
//! otherwise the file is frozen on the last captured frame. So a live view
//! needs two things, both held for the panel's lifetime:
//!
//! 1. **Wake** — open the printer's bespoke per-device mTLS MQTT control
//!    plane ([`SnapMonitorSession`]) and publish `camera.start_monitor`;
//!    release with `camera.stop_monitor` on teardown. The session must
//!    stay subscribed to `<sn>/response` the whole time — the daemon only
//!    keeps emitting for a present, authorized client.
//! 2. **Poll** — the generic Moonraker JPEG poll
//!    (`super::super::moonraker::webcam::poll_frame`) against
//!    [`monitor_url`]; only the wake is Snapmaker-specific.
//!
//! Faithful port of `iksteen/machin3d-overlay`'s `video/u1_camera.rs`.
//! The driving of these from a [`CameraSource`] lives in
//! `driver/camera.rs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rumqttc::{AsyncClient, ConnectReturnCode, Event, EventLoop, MqttOptions, Packet, QoS};
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::mtls;
use super::snap_token::SnapToken;
use crate::core::driver::traits::DriverError;

/// The `domain` the daemon recognizes for LAN monitor mode — the literal
/// `"lan"`; other identifiers are rejected.
const MONITOR_DOMAIN: &str = "lan";
const MQTT_KEEPALIVE: Duration = Duration::from_secs(30);
const MQTT_CONNECT_BUDGET: Duration = Duration::from_secs(8);
const MQTT_RESPONSE_BUDGET: Duration = Duration::from_secs(10);
/// Tight budget for the stop_monitor cleanup so teardown never drags.
const STOP_MONITOR_BUDGET: Duration = Duration::from_secs(3);

/// The HTTP URL the monitor frame is polled from. Despite the daemon
/// returning a `url` field, the path Orca actually polls keeps the
/// `/server/` prefix (verified from a packet capture in the reference).
pub fn monitor_url(host: &str, port: u16) -> String {
    format!("http://{host}:{port}/server/files/camera/monitor.jpg")
}

type PendingResponses = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

/// A live mTLS MQTT session against the printer's control plane, holding
/// monitor mode open. The daemon only routes `<sn>/request` to itself for
/// clients actively subscribed to `<sn>/response`, so we keep the session
/// (and its event-loop driver task) alive for the whole view.
pub struct SnapMonitorSession {
    client: AsyncClient,
    request_topic: String,
    driver: JoinHandle<()>,
    pending: PendingResponses,
}

impl SnapMonitorSession {
    async fn connect(token: &SnapToken) -> Result<Self, DriverError> {
        let sn = token.sn.as_str();
        let mut options =
            MqttOptions::new(token.clientid.clone(), token.host.clone(), token.mqtt_port);
        options.set_keep_alive(MQTT_KEEPALIVE);
        options.set_clean_session(true);
        options.set_transport(mtls::transport_for(token)?);

        let (client, mut eventloop) = AsyncClient::new(options, 32);

        // The daemon won't route our request unless we're a present
        // subscriber of `<sn>/response`.
        let response_topic = format!("{sn}/response");
        client
            .subscribe(response_topic.clone(), QoS::AtMostOnce)
            .await
            .map_err(|e| DriverError::Network(format!("subscribe {response_topic}: {e}")))?;

        wait_for_connack(&mut eventloop).await?;

        let pending: PendingResponses = Arc::new(Mutex::new(HashMap::new()));
        let driver = tokio::spawn(drive_eventloop(
            eventloop,
            response_topic,
            Arc::clone(&pending),
        ));

        Ok(Self {
            client,
            request_topic: format!("{sn}/request"),
            driver,
            pending,
        })
    }

    async fn start_monitor(&self) -> Result<(), DriverError> {
        let response = self.invoke("camera.start_monitor").await?;
        let state = response
            .get("result")
            .and_then(|r| r.get("state"))
            .and_then(Value::as_str);
        match state {
            Some("success") => Ok(()),
            other => Err(DriverError::Protocol(format!(
                "camera.start_monitor returned state {other:?} (expected `success`)"
            ))),
        }
    }

    async fn stop_monitor(&self) -> Result<(), DriverError> {
        self.invoke("camera.stop_monitor").await.map(|_| ())
    }

    /// Stop monitor mode and tear the session down within a tight budget.
    pub async fn release(self) {
        match tokio::time::timeout(STOP_MONITOR_BUDGET, self.stop_monitor()).await {
            Ok(Ok(())) => tracing::debug!("U1 camera monitor stopped"),
            Ok(Err(e)) => tracing::warn!(error = %e, "U1 camera stop_monitor failed"),
            Err(_) => tracing::warn!("U1 camera stop_monitor timed out"),
        }
        let _ = self.client.disconnect().await;
        self.driver.abort();
        let _ = self.driver.await;
    }

    /// Publish `<method>` on `<sn>/request` and await the matching reply on
    /// `<sn>/response`.
    async fn invoke(&self, method: &str) -> Result<Value, DriverError> {
        let req_id = unix_millis_id();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(req_id, tx);

        let payload = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": {"domain": MONITOR_DOMAIN, "interval": 0, "expect_pw": true},
            "id": req_id,
        }))
        .map_err(|e| DriverError::Other(format!("encode {method}: {e}")))?;

        if let Err(e) = self
            .client
            .publish(self.request_topic.clone(), QoS::AtLeastOnce, false, payload)
            .await
        {
            self.pending.lock().expect("pending lock").remove(&req_id);
            return Err(DriverError::Network(format!("publish {method}: {e}")));
        }

        match tokio::time::timeout(MQTT_RESPONSE_BUDGET, rx).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err(DriverError::Other(format!(
                "response channel dropped for {method}"
            ))),
            Err(_) => {
                self.pending.lock().expect("pending lock").remove(&req_id);
                Err(DriverError::Network(format!(
                    "timed out after {}s waiting for {method}",
                    MQTT_RESPONSE_BUDGET.as_secs()
                )))
            }
        }
    }
}

/// Wake the camera daemon: open the mTLS session and start monitor mode.
/// Best-effort — a failure logs and returns `None`, and the caller polls
/// anyway (the daemon may already be awake from a print or another
/// client). The returned session must be held for the view's lifetime and
/// released via [`SnapMonitorSession::release`].
pub async fn wake(token: &SnapToken) -> Option<SnapMonitorSession> {
    let session = match SnapMonitorSession::connect(token).await {
        Ok(session) => session,
        Err(e) => {
            tracing::warn!(error = %e, "U1 camera: could not open mTLS session; polling without waking");
            return None;
        }
    };
    if let Err(e) = session.start_monitor().await {
        tracing::warn!(error = %e, "U1 camera start_monitor failed; polling anyway");
    } else {
        tracing::info!(sn = %token.sn, "U1 camera monitor started");
    }
    Some(session)
}

fn unix_millis_id() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

async fn wait_for_connack(eventloop: &mut EventLoop) -> Result<(), DriverError> {
    let deadline = tokio::time::Instant::now() + MQTT_CONNECT_BUDGET;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(DriverError::Network(format!(
                "timed out after {}s waiting for U1 mTLS CONNACK",
                MQTT_CONNECT_BUDGET.as_secs()
            )));
        }
        let event = tokio::time::timeout(remaining, eventloop.poll())
            .await
            .map_err(|_| DriverError::Network("timed out waiting for U1 mTLS CONNACK".to_owned()))?
            .map_err(|e| DriverError::Network(format!("U1 mTLS connect: {e}")))?;
        if let Event::Incoming(Packet::ConnAck(ack)) = event {
            if ack.code != ConnectReturnCode::Success {
                return Err(DriverError::Auth(format!(
                    "U1 mTLS CONNECT rejected: {:?}",
                    ack.code
                )));
            }
            return Ok(());
        }
    }
}

async fn drive_eventloop(
    mut eventloop: EventLoop,
    response_topic: String,
    pending: PendingResponses,
) {
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::Publish(publish))) if publish.topic == response_topic => {
                deliver_response(&publish.payload, &pending);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::debug!(error = %e, "U1 mTLS event loop ended");
                return;
            }
        }
    }
}

fn deliver_response(payload: &[u8], pending: &PendingResponses) {
    let Ok(value) = serde_json::from_slice::<Value>(payload) else {
        return;
    };
    let Some(id) = value.get("id").and_then(Value::as_u64) else {
        return;
    };
    if let Some(sender) = pending.lock().expect("pending lock").remove(&id) {
        let _ = sender.send(value);
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
}
