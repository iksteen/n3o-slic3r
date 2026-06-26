//! [`U1Driver`] — the [`Driver`] trait impl that stitches together
//! the WebSocket session, status decoder, and HTTP control plane.
//!
//! Architecture:
//!
//! - One background task per driver. The worker owns a
//!   [`MoonrakerSession`] (rebuilt on every reconnect), decodes each
//!   incoming status snapshot via [`super::status::decode`], and
//!   publishes the result through a `watch::Sender<PrinterStatus>`.
//! - Reconnect is the driver's concern (not the session's): on any
//!   session failure / clean close, the worker waits with the shared
//!   exponential backoff (capped at 60s, reset on success) and tries
//!   again. Each connect attempt is bounded by `CONNECT_TIMEOUT` so an
//!   unreachable host fails fast into backoff instead of hanging. See
//!   [`crate::core::driver::backoff`].
//! - `send` + `command` reuse the same HTTP host/port; they don't
//!   need the session at all.
//!
//! Shape mirrors `core/driver/bambu/connection.rs` so the trait
//! surface stays consistent across drivers.

use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tracing::warn;

use super::{http, moonraker::MoonrakerSession, probe, status as status_decode};
use crate::core::driver::backoff::{reconnect_backoff_secs, CONNECT_TIMEOUT};
use crate::core::driver::status::{ConnectionState, DriverExtra, PrinterStatus, U1Extra};
use crate::core::driver::traits::{
    Driver, DriverError, DriverId, DriverKind, PrinterCommand, SendHandle, SendPayload,
    UploadProgressFn,
};

/// Per-driver connection config. Pulled out of
/// [`crate::core::driver::traits::DriverConfig::U1`] at registry-
/// register time so the trait surface stays variant-free.
#[derive(Debug, Clone)]
pub struct U1Config {
    pub host: String,
    pub port: u16,
}

pub struct U1Driver {
    id: DriverId,
    config: U1Config,
    /// Resolved serial — populated by `connect()` via the
    /// `/machine/system_info` probe.
    serial: Option<String>,
    /// Status publisher. Cloned across `subscribe_status` callers.
    status_tx: watch::Sender<PrinterStatus>,
    status_rx: watch::Receiver<PrinterStatus>,
    /// Worker JoinHandle — held so `disconnect()` / `drop()` can
    /// abort it cleanly. `Vec` to match the bambu shape; we only
    /// ever push one task today.
    tasks: Vec<JoinHandle<()>>,
    /// Signals the worker to stop without waiting for the current
    /// session to complete a round trip.
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl U1Driver {
    pub fn new(id: DriverId, config: U1Config) -> Self {
        let initial = PrinterStatus::disconnected_for(DriverExtra::U1(U1Extra::default()));
        let (status_tx, status_rx) = watch::channel(initial);
        Self {
            id,
            config,
            serial: None,
            status_tx,
            status_rx,
            tasks: Vec::new(),
            shutdown_tx: None,
        }
    }

    /// The serial-derived device id, available after a successful
    /// `connect()`. Useful for cross-printer correlation in
    /// multi-instance projects (leg 2 territory).
    #[allow(dead_code)] // surfaced once the UI needs it
    pub fn serial(&self) -> Option<&str> {
        self.serial.as_deref()
    }

    fn publish_state(&self, state: ConnectionState) {
        self.status_tx.send_modify(|s| {
            s.connection = state;
            s.last_updated = std::time::SystemTime::now();
        });
    }
}

#[async_trait]
impl Driver for U1Driver {
    fn id(&self) -> DriverId {
        self.id
    }

    fn kind(&self) -> DriverKind {
        DriverKind::U1
    }

    async fn connect(&mut self) -> Result<(), DriverError> {
        if !self.tasks.is_empty() {
            // Already connected; idempotent.
            return Ok(());
        }
        self.publish_state(ConnectionState::Connecting);

        // Probe the serial via /machine/system_info. We fail loud
        // here rather than letting the worker race — a wrong host
        // yields a clean "could not reach printer at <host>" instead
        // of "connecting…" stuck forever.
        let serial = probe::probe_system_info(&self.config.host, self.config.port)
            .await?
            .serial;
        self.serial = Some(serial);

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);
        let host = self.config.host.clone();
        let port = self.config.port;
        let status_tx = self.status_tx.clone();
        let task = tokio::spawn(run_worker(host, port, status_tx, shutdown_rx));
        self.tasks.push(task);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), DriverError> {
        if let Some(tx) = self.shutdown_tx.take() {
            // Ignore send error: receiver may already be gone if
            // the worker terminated naturally — disconnect is a
            // no-op in that case.
            let _ = tx.send(());
        }
        for task in self.tasks.drain(..) {
            task.abort();
            // Don't await — abort is fire-and-forget; awaiting
            // would block disconnect on the worker's scheduler
            // tick. The watch sender drops with `self` and any
            // subscribers see the channel close.
            drop(task);
        }
        self.publish_state(ConnectionState::Disconnected {
            reason: "disconnect() called".into(),
        });
        Ok(())
    }

    fn status(&self) -> PrinterStatus {
        self.status_rx.borrow().clone()
    }

    fn subscribe_status(&self) -> watch::Receiver<PrinterStatus> {
        self.status_rx.clone()
    }

    async fn send(
        &mut self,
        payload: SendPayload,
        on_progress: UploadProgressFn,
    ) -> Result<SendHandle, DriverError> {
        match payload {
            SendPayload::Gcode { bytes, file_name } => {
                http::upload_and_start(
                    &self.config.host,
                    self.config.port,
                    &file_name,
                    bytes,
                    on_progress,
                )
                .await
            }
            SendPayload::Gcode3mf { .. } => Err(DriverError::Other(
                "U1 expects raw G-code (SendPayload::Gcode); .gcode.3mf is Bambu-only".into(),
            )),
        }
    }

    async fn command(&mut self, cmd: PrinterCommand) -> Result<(), DriverError> {
        http::send_command(&self.config.host, self.config.port, cmd).await
    }
}

/// Background task: hold one [`MoonrakerSession`] at a time, decode
/// every incoming snapshot, publish through `status_tx`. Reconnects
/// with exponential backoff on session failure or clean close.
async fn run_worker(
    host: String,
    port: u16,
    status_tx: watch::Sender<PrinterStatus>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let mut attempt = 0u32;
    loop {
        status_tx.send_modify(|s| {
            s.connection = ConnectionState::Connecting;
            s.last_updated = std::time::SystemTime::now();
        });

        // Bound the connect (WS handshake + initial subscribe) so an
        // unreachable host fails fast into backoff instead of stalling
        // on the OS TCP timeout.
        let session = tokio::select! {
            r = tokio::time::timeout(CONNECT_TIMEOUT, MoonrakerSession::connect(&host, port)) => {
                match r {
                    Ok(inner) => inner,
                    Err(_) => Err(DriverError::Network(format!(
                        "connect timed out after {}s",
                        CONNECT_TIMEOUT.as_secs()
                    ))),
                }
            }
            _ = &mut shutdown_rx => return,
        };

        // The reason the session ended — threaded into the
        // Reconnecting status so the UI / test-connection can report
        // why rather than a bare countdown.
        let reason = match session {
            Ok(mut session) => {
                // Reset backoff on every successful connect — a
                // healthy printer that disconnects briefly should
                // come back fast on the next round.
                attempt = 0;
                // Publish the initial subscribe response so the UI
                // doesn't sit on a stale "Connecting…" snapshot.
                let initial = status_decode::decode(&session.status(), ConnectionState::Connected);
                let _ = status_tx.send_replace(initial);

                loop {
                    let next = tokio::select! {
                        r = session.next_status() => r,
                        _ = &mut shutdown_rx => return,
                    };
                    match next {
                        Ok(Some(snapshot)) => {
                            let decoded =
                                status_decode::decode(&snapshot, ConnectionState::Connected);
                            let _ = status_tx.send_replace(decoded);
                        }
                        Ok(None) => {
                            // Clean server close → reconnect cycle.
                            break "the printer closed the connection".to_string();
                        }
                        Err(e) => {
                            warn!(?e, host = %host, port, "Moonraker session failed");
                            // `e` is a DriverError whose Display already
                            // names the cause (network/auth/protocol) —
                            // surface it verbatim, matching the Bambu side
                            // rather than double-prefixing.
                            break e.to_string();
                        }
                    }
                }
            }
            Err(e) => {
                warn!(?e, host = %host, port, "Moonraker connect failed");
                e.to_string()
            }
        };

        // Publish the upcoming reconnect window so the UI can show
        // a countdown rather than a generic "disconnected".
        let delay = reconnect_backoff_secs(attempt);
        attempt = attempt.saturating_add(1);
        status_tx.send_modify(|s| {
            s.connection = ConnectionState::Reconnecting {
                in_seconds: delay as u32,
                reason,
            };
            s.last_updated = std::time::SystemTime::now();
        });

        let sleep = tokio::time::sleep(Duration::from_secs(delay));
        tokio::pin!(sleep);
        tokio::select! {
            _ = &mut sleep => {}
            _ = &mut shutdown_rx => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::driver::status::JobState;
    use crate::core::driver::traits::DriverId;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio_tungstenite::tungstenite::Message;

    /// Local mock Moonraker server. Serves two endpoints on the same
    /// port: the HTTP `GET /machine/system_info` probe (driven by
    /// `probe_serial` — `Some` → 200 with that serial, `None` → 404 so
    /// the probe fails cleanly) and the status WebSocket. The WS path
    /// replies to the `printer.objects.subscribe` request with
    /// `initial_status`, then optionally pushes one
    /// `notify_status_update`. Bound to an ephemeral port so tests
    /// don't race. Connections are dispatched by peeking the request
    /// line, so the same listener handles the connect-time probe and
    /// the subsequent WS worker.
    async fn start_mock_moonraker(
        initial_status: serde_json::Value,
        notify: Option<serde_json::Value>,
        probe_serial: Option<&str>,
    ) -> (String, u16, oneshot::Sender<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let probe_serial = probe_serial.map(str::to_owned);
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                let accept = tokio::select! {
                    r = listener.accept() => r,
                    _ = &mut stop_rx => return,
                };
                let Ok((stream, _peer)) = accept else {
                    continue;
                };
                let initial_status = initial_status.clone();
                let notify = notify.clone();
                let probe_serial = probe_serial.clone();
                tokio::spawn(async move {
                    // Dispatch on the request line without consuming it
                    // (TcpStream::peek leaves the bytes for the WS
                    // handshake). The HTTP probe targets
                    // `/machine/system_info`; the worker opens `GET /`.
                    let mut peek = [0u8; 256];
                    let n = match stream.peek(&mut peek).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    if String::from_utf8_lossy(&peek[..n]).starts_with("GET /machine/system_info") {
                        use tokio::io::AsyncWriteExt;
                        let (status_line, body) = match &probe_serial {
                            Some(serial) => (
                                "200 OK",
                                json!({ "result": { "system_info": { "product_info": {
                                    "serial_number": serial,
                                    "device_name": "Mock U1",
                                }}}})
                                .to_string(),
                            ),
                            None => ("404 Not Found", String::new()),
                        };
                        let resp = format!(
                            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body,
                        );
                        let mut stream = stream;
                        let _ = stream.write_all(resp.as_bytes()).await;
                        let _ = stream.flush().await;
                        return;
                    }
                    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    // First message must be a subscribe request; echo
                    // its `id` back so the client's `send_subscribe`
                    // path completes.
                    let req_id = loop {
                        let Some(msg) = ws.next().await else { return };
                        let Ok(Message::Text(text)) = msg else {
                            continue;
                        };
                        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                        if v.get("method").and_then(|m| m.as_str())
                            == Some("printer.objects.subscribe")
                        {
                            break v.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
                        }
                    };
                    let response = json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": { "status": initial_status },
                    });
                    let _ = ws.send(Message::Text(response.to_string())).await;
                    if let Some(update) = notify {
                        let notify = json!({
                            "jsonrpc": "2.0",
                            "method": "notify_status_update",
                            "params": [update, 0.0],
                        });
                        let _ = ws.send(Message::Text(notify.to_string())).await;
                    }
                    // Keep the connection open until the driver
                    // disconnects (otherwise the worker would race
                    // into a reconnect cycle and flap the status).
                    while let Some(msg) = ws.next().await {
                        if matches!(msg, Ok(Message::Close(_))) || msg.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (addr.ip().to_string(), addr.port(), stop_tx)
    }

    /// Wait for `predicate` to return Some on the driver's published
    /// status, or fail the test after `timeout`. Polls the watch
    /// `changed()` signal so we don't hot-spin.
    async fn wait_for<F, T>(driver: &U1Driver, timeout: Duration, mut predicate: F) -> T
    where
        F: FnMut(&PrinterStatus) -> Option<T>,
    {
        let mut rx = driver.subscribe_status();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(value) = predicate(&rx.borrow()) {
                return value;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                panic!(
                    "wait_for timed out after {timeout:?}; last status = {:?}",
                    *rx.borrow()
                );
            }
            if tokio::time::timeout(remaining, rx.changed()).await.is_err() {
                panic!(
                    "wait_for timed out after {timeout:?}; last status = {:?}",
                    *rx.borrow()
                );
            }
        }
    }

    #[tokio::test]
    async fn connect_publishes_initial_then_streamed_status() {
        let initial = json!({
            "print_stats": { "state": "standby", "filename": "" }
        });
        let update = json!({
            "print_stats": { "state": "printing", "filename": "Cube.gcode" }
        });
        let (host, port, _stop) =
            start_mock_moonraker(initial, Some(update), Some("mock-serial")).await;
        let mut driver = U1Driver::new(DriverId(99), U1Config { host, port });
        driver.connect().await.expect("connect");

        // Wait for the streamed update to land — implies both the
        // initial subscribe response and the notify_status_update
        // were decoded + published.
        let job = wait_for(&driver, Duration::from_secs(3), |s| {
            s.job
                .as_ref()
                .filter(|j| j.file_name.as_deref() == Some("Cube.gcode"))
                .cloned()
        })
        .await;
        assert!(matches!(job.state, JobState::Printing));

        driver.disconnect().await.expect("disconnect");
    }

    #[tokio::test]
    async fn connect_probes_system_info_and_surfaces_probe_failure() {
        // connect() always probes /machine/system_info before opening
        // the WS. Here the mock returns 404 for the probe, so the
        // driver should surface that error cleanly without spawning a
        // worker.
        let (host, port, _stop) = start_mock_moonraker(json!({}), None, None).await;
        let mut driver = U1Driver::new(DriverId(100), U1Config { host, port });
        let err = driver.connect().await.unwrap_err();
        assert!(
            matches!(err, DriverError::Network(_) | DriverError::Protocol(_)),
            "{err:?}",
        );
        // No worker should have been spawned on a failed probe.
        assert!(driver.tasks.is_empty());
    }

    #[tokio::test]
    async fn disconnect_is_idempotent_and_clears_tasks() {
        let (host, port, _stop) = start_mock_moonraker(
            json!({ "print_stats": { "state": "standby" } }),
            None,
            Some("mock"),
        )
        .await;
        let mut driver = U1Driver::new(DriverId(101), U1Config { host, port });
        driver.connect().await.unwrap();
        // First disconnect tears down.
        driver.disconnect().await.unwrap();
        assert!(driver.tasks.is_empty());
        // Second disconnect is a no-op.
        driver.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn send_rejects_gcode3mf_payload_with_clear_message() {
        let mut driver = U1Driver::new(
            DriverId(102),
            U1Config {
                host: "127.0.0.1".into(),
                port: 1,
            },
        );
        // Driver doesn't need to be connected for this — the variant
        // check is up-front. Use a placeholder Bambu payload.
        let payload = SendPayload::Gcode3mf {
            bytes: vec![],
            plate_id: 1,
            file_basename: "untitled_Plate_1".into(),
            use_ams: false,
            ams_mapping: Vec::new(),
            ams_mapping2: Vec::new(),
        };
        let err = driver
            .send(payload, std::sync::Arc::new(|_, _| {}))
            .await
            .unwrap_err();
        match err {
            DriverError::Other(msg) => {
                assert!(msg.contains("U1"));
                assert!(msg.contains(".gcode.3mf"));
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }
}
