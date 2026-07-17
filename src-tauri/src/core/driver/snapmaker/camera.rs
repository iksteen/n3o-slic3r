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

use futures_util::{SinkExt, StreamExt};
use rumqttc::{AsyncClient, ConnectReturnCode, Event, EventLoop, MqttOptions, Packet, QoS};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;

use super::mtls;
use super::snap_token::SnapToken;
use crate::core::driver::traits::DriverError;

/// The `domain` the daemon recognizes for LAN monitor mode — the literal
/// `"lan"`; other identifiers are rejected.
const MONITOR_DOMAIN: &str = "lan";
/// The daemon's capture watchdog hard-stops monitor mode at ~361s unless a
/// fresh `start_monitor` resets it; re-send comfortably inside that window.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
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
    heartbeat: JoinHandle<()>,
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
            client.clone(),
            response_topic,
            Arc::clone(&pending),
        ));

        let request_topic = format!("{sn}/request");
        let heartbeat = tokio::spawn(heartbeat_loop(
            client.clone(),
            request_topic.clone(),
            Arc::clone(&pending),
        ));

        Ok(Self {
            client,
            request_topic,
            driver,
            heartbeat,
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
        self.heartbeat.abort();
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
        invoke(&self.client, &self.request_topic, &self.pending, method).await
    }
}

/// Publish `<method>` on `<request_topic>` and await the matching reply,
/// correlated by request id via `pending`. Shared by the session's own
/// calls and the keep-alive heartbeat.
async fn invoke(
    client: &AsyncClient,
    request_topic: &str,
    pending: &PendingResponses,
    method: &str,
) -> Result<Value, DriverError> {
    let req_id = unix_millis_id();
    let (tx, rx) = oneshot::channel();
    pending.lock().expect("pending lock").insert(req_id, tx);

    let payload = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": {"domain": MONITOR_DOMAIN, "interval": 0, "expect_pw": true},
        "id": req_id,
    }))
    .map_err(|e| DriverError::Other(format!("encode {method}: {e}")))?;

    if let Err(e) = client
        .publish(request_topic.to_owned(), QoS::AtLeastOnce, false, payload)
        .await
    {
        pending.lock().expect("pending lock").remove(&req_id);
        return Err(DriverError::Network(format!("publish {method}: {e}")));
    }

    match tokio::time::timeout(MQTT_RESPONSE_BUDGET, rx).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) => Err(DriverError::Other(format!(
            "response channel dropped for {method}"
        ))),
        Err(_) => {
            pending.lock().expect("pending lock").remove(&req_id);
            Err(DriverError::Network(format!(
                "timed out after {}s waiting for {method}",
                MQTT_RESPONSE_BUDGET.as_secs()
            )))
        }
    }
}

/// Keep monitor mode alive: re-send `camera.start_monitor` every
/// [`HEARTBEAT_INTERVAL`] to reset the daemon's ~361s capture watchdog.
/// Runs until aborted on session release.
async fn heartbeat_loop(client: AsyncClient, request_topic: String, pending: PendingResponses) {
    loop {
        tokio::time::sleep(HEARTBEAT_INTERVAL).await;
        match invoke(&client, &request_topic, &pending, "camera.start_monitor").await {
            Ok(_) => tracing::trace!("U1 camera monitor heartbeat refreshed"),
            Err(e) => tracing::warn!(error = %e, "U1 camera monitor heartbeat failed"),
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

// ---- No-pairing LAN path (§0): wake over Moonraker's open WebSocket ----

type WsSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A WebSocket wake holding monitor mode open on the no-pairing LAN path.
/// One task re-sends `camera.start_monitor` on the [`HEARTBEAT_INTERVAL`]
/// and drains incoming frames (so tungstenite answers pings); [`release`]
/// cancels it, which sends `camera.stop_monitor` before the socket drops.
pub struct WsMonitorSession {
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl WsMonitorSession {
    /// Stop monitor mode and tear the session down.
    pub async fn release(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }
}

/// Wake the camera over Moonraker's open WebSocket JSON-RPC — the
/// no-pairing LAN path. On the U1's stock config any LAN IP is a trusted
/// client, so `camera.start_monitor` needs no cert and no API key. The
/// call is fire-and-forget (Moonraker's repeater returns null); the frame
/// URL is the fixed monitor path regardless. Best-effort like [`wake`].
pub async fn wake_ws(host: &str, port: u16) -> Option<WsMonitorSession> {
    let url = format!("ws://{host}:{port}/websocket");
    let request = match url.as_str().into_client_request() {
        Ok(request) => request,
        Err(e) => {
            tracing::warn!(error = %e, "U1 camera: bad WS URL; polling without waking");
            return None;
        }
    };
    let socket = match connect_async(request).await {
        Ok((socket, _)) => socket,
        Err(e) => {
            tracing::warn!(error = %e, "U1 camera: WS connect failed; polling without waking");
            return None;
        }
    };
    tracing::info!(host, "U1 camera monitor started (WS)");
    let cancel = CancellationToken::new();
    let task = tokio::spawn(drive_ws_monitor(socket, cancel.clone()));
    Some(WsMonitorSession { cancel, task })
}

async fn drive_ws_monitor(mut socket: WsSocket, cancel: CancellationToken) {
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let _ = send_ws_camera(&mut socket, "camera.stop_monitor").await;
                break;
            }
            // interval's first tick fires immediately → the initial wake;
            // every tick after resets the daemon's ~361s watchdog.
            _ = heartbeat.tick() => {
                if let Err(e) = send_ws_camera(&mut socket, "camera.start_monitor").await {
                    tracing::warn!(error = %e, "U1 camera WS heartbeat failed");
                }
            }
            message = socket.next() => match message {
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    tracing::debug!(error = %e, "U1 camera WS read error; ending session");
                    break;
                }
                None => break,
            },
        }
    }
}

/// Send a `camera.*` JSON-RPC request over the WebSocket. The §0 path is
/// fire-and-forget — the repeater returns null — so we don't await a reply.
/// Uses the dotted method name (`camera/…` 404s as method-not-found).
async fn send_ws_camera(socket: &mut WsSocket, method: &str) -> Result<(), DriverError> {
    let payload = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": {"domain": MONITOR_DOMAIN, "interval": 1, "expect_pw": false},
        "id": unix_millis_id(),
    })
    .to_string();
    socket
        .send(Message::Text(payload.into()))
        .await
        .map_err(|e| DriverError::Network(format!("WS send {method}: {e}")))
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

/// Drive the mTLS session's event loop for the whole view. Polling
/// *through* errors is deliberate: rumqttc reconnects on the next poll
/// after a drop, so a transient disconnect must not end this task — if it
/// did, the client's request channel would break and every heartbeat
/// publish would fail with "Failed to send mqtt requests to eventloop".
/// `clean_session` drops subscriptions on reconnect, so we re-subscribe to
/// `<sn>/response` on each ConnAck (the first ConnAck is consumed by
/// `wait_for_connack`, so this only fires on reconnects). The task is
/// aborted by [`SnapMonitorSession::release`] when the view closes.
async fn drive_eventloop(
    mut eventloop: EventLoop,
    client: AsyncClient,
    response_topic: String,
    pending: PendingResponses,
) {
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(ack))) if ack.code == ConnectReturnCode::Success => {
                if let Err(e) = client
                    .subscribe(response_topic.clone(), QoS::AtMostOnce)
                    .await
                {
                    tracing::warn!(error = %e, "U1 camera: re-subscribe after reconnect failed");
                }
            }
            Ok(Event::Incoming(Packet::Publish(publish))) if publish.topic == response_topic => {
                deliver_response(&publish.payload, &pending);
            }
            Ok(_) => {}
            Err(e) => {
                // Reconnect happens on the next poll; back off so a hard-down
                // printer doesn't spin this into a hot loop.
                tracing::debug!(error = %e, "U1 mTLS event loop error; will retry");
                tokio::time::sleep(Duration::from_secs(1)).await;
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
