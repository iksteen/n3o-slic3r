//! [`BambuDriver`] — `Driver` impl + rumqttc lifecycle.
//!
//! On `connect()`:
//!   1. Run [`super::device_id::probe`] to get the printer's
//!      serial from the peer cert.
//!   2. Spawn a background task that owns the rumqttc event
//!      loop. The task subscribes to `device/<id>/report`,
//!      publishes a `pushall` request, then forwards every
//!      incoming `Publish` payload to an `mpsc::Sender<Vec<u8>>`
//!      for PR-7a-3's status parser.
//!   3. The task also pushes connection-state transitions into
//!      a `watch::Sender<PrinterStatus>` the driver's
//!      `subscribe_status` exposes.
//!
//! On disconnect: signal the task via a oneshot channel; it
//! tears down the rumqttc client + exits.
//!
//! Reconnect: exponential backoff (1, 2, 4, 8, 16 sec, cap 60s).
//! During backoff the status connection state is
//! `Reconnecting { in_seconds }` so the UI can show progress.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, Transport};
use serde::Serialize;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::core::driver::status::{
    BambuExtra, ConnectionState, DriverExtra, PrinterStatus,
};
use crate::core::driver::traits::{
    Driver, DriverError, DriverId, DriverKind, PrinterCommand, SendHandle, SendPayload,
};

const KEEPALIVE: Duration = Duration::from_secs(60);
const RECONNECT_BACKOFFS: &[u32] = &[1, 2, 4, 8, 16];
const RECONNECT_CAP_SECS: u32 = 60;

/// Connection-level configuration. The serial is optional —
/// when `None`, the connect path probes the peer cert CN.
#[derive(Debug, Clone)]
pub struct BambuConfig {
    pub host: String,
    pub access_code: String,
    pub serial: Option<String>,
}

/// The driver itself. Stored in the [`super::super::DriverRegistry`]
/// as `Box<dyn Driver>`.
pub struct BambuDriver {
    id: DriverId,
    config: BambuConfig,
    /// Resolved serial — populated by `connect()`.
    device_id: Option<String>,
    /// Status publisher. Cloned across `subscribe_status` callers.
    status_tx: watch::Sender<PrinterStatus>,
    status_rx: watch::Receiver<PrinterStatus>,
    /// Sink that the background task pushes raw report payloads
    /// into. PR-7a-3's parser drains it. `None` until connected.
    raw_messages_rx: Option<Arc<Mutex<mpsc::Receiver<Vec<u8>>>>>,
    /// Handle to the spawned background tasks — held so `drop()`
    /// or `disconnect()` can abort them. Two tasks per driver:
    /// the rumqttc event loop (PR-7a-2) and the status worker
    /// (PR-7a-3).
    tasks: Vec<JoinHandle<()>>,
    /// Signals the event-loop task to stop cleanly.
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// Client handle for publishing (PR-7a-5 / PR-7a-6 will use
    /// this to send print + command messages).
    client: Option<AsyncClient>,
}

impl BambuDriver {
    pub fn new(id: DriverId, config: BambuConfig) -> Self {
        let initial = PrinterStatus::disconnected_for(DriverExtra::Bambu(
            BambuExtra::default(),
        ));
        let (status_tx, status_rx) = watch::channel(initial);
        Self {
            id,
            config,
            device_id: None,
            status_tx,
            status_rx,
            raw_messages_rx: None,
            tasks: Vec::new(),
            shutdown_tx: None,
            client: None,
        }
    }

    /// The serial-derived device id, available after a
    /// successful `connect()`. Used by PR-7a-3 (status parser),
    /// PR-7a-5 (send-print MQTT command), and PR-7a-6 (commands).
    #[allow(dead_code)] // consumed by PR-7a-3..-6
    pub fn device_id(&self) -> Option<&str> {
        self.device_id.as_deref()
    }

    /// Channel the parser drains. PR-7a-3 wires this into its
    /// status worker on driver setup.
    #[allow(dead_code)] // consumed by PR-7a-3
    pub fn raw_messages(&self) -> Option<Arc<Mutex<mpsc::Receiver<Vec<u8>>>>> {
        self.raw_messages_rx.clone()
    }

    fn set_connection_state(&self, state: ConnectionState) {
        self.status_tx.send_modify(|s| {
            s.connection = state;
            s.last_updated = std::time::SystemTime::now();
        });
    }
}

#[async_trait]
impl Driver for BambuDriver {
    fn id(&self) -> DriverId {
        self.id
    }

    fn kind(&self) -> DriverKind {
        DriverKind::Bambu
    }

    async fn connect(&mut self) -> Result<(), DriverError> {
        if !self.tasks.is_empty() {
            // Idempotent: already connected.
            return Ok(());
        }
        self.set_connection_state(ConnectionState::Connecting);

        // Probe the serial unless the caller already supplied one.
        let device_id = if let Some(s) = &self.config.serial {
            s.clone()
        } else {
            super::device_id::probe(&self.config.host, 8883).await?
        };
        self.device_id = Some(device_id.clone());

        // rumqttc options.
        let mut options = MqttOptions::new(
            format!("n3o-slic3r-{}", Uuid::new_v4()),
            &self.config.host,
            8883,
        );
        options.set_keep_alive(KEEPALIVE);
        options.set_credentials("bblp", &self.config.access_code);
        let connector = super::tls::connector().map_err(DriverError::Other)?;
        options.set_transport(Transport::tls_with_config(connector.into()));

        let (client, eventloop) = AsyncClient::new(options, 32);
        self.client = Some(client.clone());

        // Two channels: rumqttc loop pushes raw payloads into
        // raw_tx; the status worker drains raw_rx, parses,
        // merges, and emits to the watch sender.
        let (raw_tx, raw_rx) = mpsc::channel::<Vec<u8>>(64);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        // Status worker task (PR-7a-3). We keep no handle to its
        // mpsc receiver because the worker owns it; the
        // raw_messages_rx field is left None until a future
        // ticket needs out-of-band access to raw payloads.
        let _ = &self.raw_messages_rx;

        let status_tx_for_worker = self.status_tx.clone();
        let worker_task = tokio::spawn(super::status::run_worker(
            raw_rx,
            status_tx_for_worker,
        ));
        self.tasks.push(worker_task);

        // rumqttc event loop task (PR-7a-2).
        let status_tx = self.status_tx.clone();
        let device_id_owned = device_id.clone();
        let client_for_task = client.clone();
        let event_loop_task = tokio::spawn(event_loop(
            eventloop,
            client_for_task,
            device_id_owned,
            raw_tx,
            status_tx,
            shutdown_rx,
        ));
        self.tasks.push(event_loop_task);

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), DriverError> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(client) = self.client.take() {
            // Best-effort — the tasks are going down regardless.
            let _ = client.disconnect().await;
        }
        // Drain task handles. The event-loop task exits via the
        // shutdown channel; the status worker exits when its
        // mpsc receiver drops (which happens when the event-loop
        // task exits and the raw_tx sender goes out of scope).
        for handle in std::mem::take(&mut self.tasks) {
            match tokio::time::timeout(Duration::from_secs(5), handle).await {
                Ok(_) => {}
                Err(_) => {
                    tracing::warn!(driver = %self.id, "bambu task did not exit in 5s")
                }
            }
        }
        self.set_connection_state(ConnectionState::Disconnected {
            reason: "client requested".into(),
        });
        Ok(())
    }

    fn status(&self) -> PrinterStatus {
        self.status_rx.borrow().clone()
    }

    fn subscribe_status(&self) -> watch::Receiver<PrinterStatus> {
        self.status_rx.clone()
    }

    async fn send(&mut self, _payload: SendPayload) -> Result<SendHandle, DriverError> {
        // Implemented in PR-7a-5 — this is the connection ticket.
        Err(DriverError::Other(
            "BambuDriver::send not implemented yet (PR-7a-5)".into(),
        ))
    }

    async fn command(&mut self, _cmd: PrinterCommand) -> Result<(), DriverError> {
        // Implemented in PR-7a-6.
        Err(DriverError::Other(
            "BambuDriver::command not implemented yet (PR-7a-6)".into(),
        ))
    }
}

#[derive(Serialize)]
struct PushAllRequest<'a> {
    pushing: PushAllCommand<'a>,
}

#[derive(Serialize)]
struct PushAllCommand<'a> {
    sequence_id: &'a str,
    command: &'a str,
    version: u8,
    push_target: u8,
}

/// Background task: own the rumqttc event loop until shutdown.
///
/// On every iteration:
///   - Drain one event (`eventloop.poll()`).
///   - ConnAck → subscribe to the report topic + publish the
///     pushall request. Flip status to Connected.
///   - Publish to report topic → forward bytes to the parser's
///     channel.
///   - Disconnect → flip status to Reconnecting + sleep backoff
///     + retry. (rumqttc's event loop will re-attempt on its
///     own; we don't recreate the client — overlay relies on
///     the same behavior.)
///   - Shutdown signal → drain the client + exit.
async fn event_loop(
    mut eventloop: rumqttc::EventLoop,
    client: AsyncClient,
    device_id: String,
    raw_tx: mpsc::Sender<Vec<u8>>,
    status_tx: watch::Sender<PrinterStatus>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let report_topic = format!("device/{device_id}/report");
    let request_topic = format!("device/{device_id}/request");
    let mut backoff_idx = 0usize;

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                tracing::debug!(driver_serial = %device_id, "bambu task shutdown");
                return;
            }
            event = eventloop.poll() => {
                match event {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        backoff_idx = 0;
                        // Subscribe + pushall.
                        if let Err(e) = client
                            .subscribe(&report_topic, QoS::AtMostOnce)
                            .await
                        {
                            tracing::warn!(error = %e, "bambu subscribe failed");
                        }
                        let req = PushAllRequest {
                            pushing: PushAllCommand {
                                sequence_id: "0",
                                command: "pushall",
                                version: 1,
                                push_target: 1,
                            },
                        };
                        match serde_json::to_vec(&req) {
                            Ok(body) => {
                                if let Err(e) = client
                                    .publish(&request_topic, QoS::AtMostOnce, false, body)
                                    .await
                                {
                                    tracing::warn!(error = %e, "pushall publish failed");
                                }
                            }
                            Err(e) => tracing::warn!(error = %e, "pushall serialize failed"),
                        }
                        status_tx.send_modify(|s| {
                            s.connection = ConnectionState::Connected;
                            s.last_updated = std::time::SystemTime::now();
                        });
                    }
                    Ok(Event::Incoming(Packet::Publish(p))) if p.topic == report_topic => {
                        // Forward to the parser. If the channel
                        // is full, drop the oldest by closing the
                        // current send and reopening — for now,
                        // just log + drop the message on full.
                        if let Err(e) = raw_tx.try_send(p.payload.to_vec()) {
                            tracing::trace!(error = %e, "raw message channel full or closed");
                        }
                    }
                    Ok(Event::Incoming(Packet::Disconnect)) => {
                        let delay = backoff_seconds(backoff_idx);
                        backoff_idx = (backoff_idx + 1).min(RECONNECT_BACKOFFS.len());
                        status_tx.send_modify(|s| {
                            s.connection = ConnectionState::Reconnecting { in_seconds: delay };
                            s.last_updated = std::time::SystemTime::now();
                        });
                        tokio::time::sleep(Duration::from_secs(delay as u64)).await;
                    }
                    Err(e) => {
                        // Network error — rumqttc surfaces these
                        // through the same poll loop; we treat as
                        // transient + backoff.
                        let delay = backoff_seconds(backoff_idx);
                        backoff_idx = (backoff_idx + 1).min(RECONNECT_BACKOFFS.len());
                        tracing::warn!(error = %e, backoff_secs = delay, "bambu poll error");
                        status_tx.send_modify(|s| {
                            s.connection = ConnectionState::Reconnecting { in_seconds: delay };
                            s.last_updated = std::time::SystemTime::now();
                        });
                        tokio::time::sleep(Duration::from_secs(delay as u64)).await;
                    }
                    _ => {
                        // PingResp / SubAck / etc — uninteresting.
                    }
                }
            }
        }
    }
}

/// Backoff sequence: `[1, 2, 4, 8, 16]`-then-cap-at-60. Public
/// for unit testing.
pub fn backoff_seconds(attempt: usize) -> u32 {
    RECONNECT_BACKOFFS
        .get(attempt)
        .copied()
        .unwrap_or(RECONNECT_CAP_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_follows_expected_sequence() {
        assert_eq!(backoff_seconds(0), 1);
        assert_eq!(backoff_seconds(1), 2);
        assert_eq!(backoff_seconds(2), 4);
        assert_eq!(backoff_seconds(3), 8);
        assert_eq!(backoff_seconds(4), 16);
        // Beyond the array, cap at 60.
        assert_eq!(backoff_seconds(5), 60);
        assert_eq!(backoff_seconds(99), 60);
    }

    #[test]
    fn driver_constructs_in_disconnected_state() {
        let driver = BambuDriver::new(
            DriverId(1),
            BambuConfig {
                host: "192.0.2.1".into(),
                access_code: "00000000".into(),
                serial: None,
            },
        );
        match driver.status().connection {
            ConnectionState::Disconnected { .. } => {}
            other => panic!("expected Disconnected, got {other:?}"),
        }
        assert!(driver.device_id().is_none());
    }

    #[test]
    fn driver_id_and_kind_are_stable() {
        let d = BambuDriver::new(
            DriverId(7),
            BambuConfig {
                host: "x".into(),
                access_code: "y".into(),
                serial: None,
            },
        );
        assert_eq!(d.id(), DriverId(7));
        assert_eq!(d.kind(), DriverKind::Bambu);
    }

    /// Pushall payload shape is what the printer expects. Pin
    /// the JSON so a serde refactor doesn't drift the wire
    /// format without our knowing.
    #[test]
    fn pushall_request_serializes_to_expected_shape() {
        let req = PushAllRequest {
            pushing: PushAllCommand {
                sequence_id: "0",
                command: "pushall",
                version: 1,
                push_target: 1,
            },
        };
        let s = serde_json::to_string(&req).unwrap();
        assert_eq!(
            s,
            "{\"pushing\":{\"sequence_id\":\"0\",\"command\":\"pushall\",\"version\":1,\"push_target\":1}}"
        );
    }
}
