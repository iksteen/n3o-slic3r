//! [`BambuDriver`] — `Driver` impl + rumqttc lifecycle.
//!
//! On `connect()`:
//!   1. Run [`super::device_id::probe`] to get the printer's
//!      serial from the peer cert.
//!   2. Spawn a background task that owns the rumqttc event
//!      loop. The task subscribes to `device/<id>/report`,
//!      publishes a `pushall` request, then forwards every
//!      incoming `Publish` payload to an `mpsc::Sender<Vec<u8>>`
//!      for the status parser.
//!   3. The task also pushes connection-state transitions into
//!      a `watch::Sender<PrinterStatus>` the driver's
//!      `subscribe_status` exposes.
//!
//! On disconnect: signal the task via a oneshot channel; it
//! tears down the rumqttc client + exits.
//!
//! Reconnect + connect-timeout policy is shared across drivers in
//! [`crate::core::driver::backoff`]: exponential backoff capped at 60s
//! (reset on success), and each network connect bounded by
//! `CONNECT_TIMEOUT`. During backoff the status is
//! `Reconnecting { in_seconds }` so the UI can show progress.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, Transport};
use serde::Serialize;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::core::driver::backoff::{reconnect_backoff_secs, CONNECT_TIMEOUT};
use crate::core::driver::status::{
    BambuExtra, ConnectionState, DriverExtra, JobState, PrinterStatus,
};
use crate::core::driver::traits::{
    Driver, DriverError, DriverId, DriverKind, PrinterCommand, SendHandle, SendPayload,
    UploadProgressFn,
};

const KEEPALIVE: Duration = Duration::from_secs(60);

/// Connection-level configuration. The device serial is not
/// supplied here — `connect()` always probes it from the peer cert CN.
#[derive(Debug, Clone)]
pub struct BambuConfig {
    pub host: String,
    pub access_code: String,
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
    /// into. The status parser drains it. `None` until connected.
    raw_messages_rx: Option<Arc<Mutex<mpsc::Receiver<Vec<u8>>>>>,
    /// Handle to the spawned background tasks — held so `drop()`
    /// or `disconnect()` can abort them. Two tasks per driver:
    /// the rumqttc event loop and the status worker.
    tasks: Vec<JoinHandle<()>>,
    /// Signals the event-loop task to stop cleanly.
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// Client handle for publishing — used by send-print and
    /// pause/resume/stop.
    client: Option<AsyncClient>,
    /// Monotonic sequence_id counter for outgoing MQTT commands.
    /// Bambu echoes the value back in status messages so we can
    /// correlate the printer's ack with the command we sent.
    sequence_counter: Arc<AtomicU64>,
}

impl BambuDriver {
    pub fn new(id: DriverId, config: BambuConfig) -> Self {
        let initial = PrinterStatus::disconnected_for(DriverExtra::Bambu(BambuExtra::default()));
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
            sequence_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Allocate the next sequence_id for an outgoing MQTT
    /// command. Wraps as a stringified u64 — Bambu expects
    /// string-typed sequence ids.
    fn next_sequence_id(&self) -> String {
        self.sequence_counter
            .fetch_add(1, Ordering::SeqCst)
            .to_string()
    }

    /// The serial-derived device id, available after a
    /// successful `connect()`. Used by the status parser, the
    /// send-print MQTT command, and the printer commands.
    #[allow(dead_code)] // consumed by the status parser + command paths
    pub fn device_id(&self) -> Option<&str> {
        self.device_id.as_deref()
    }

    /// Channel the parser drains. The status worker wires this in
    /// on driver setup.
    #[allow(dead_code)] // consumed by the status worker
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

        // Probe the device serial from the peer cert CN.
        let device_id = super::device_id::probe(&self.config.host, 8883).await?;
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

        let (client, mut eventloop) = AsyncClient::new(options, 32);
        // Bound each network connect attempt so an unreachable host
        // fails fast into backoff instead of stalling on the OS TCP
        // timeout (shared CONNECT_TIMEOUT).
        let mut net = eventloop.network_options();
        net.set_connection_timeout(CONNECT_TIMEOUT.as_secs());
        eventloop.set_network_options(net);
        self.client = Some(client.clone());

        // Two channels: rumqttc loop pushes raw payloads into
        // raw_tx; the status worker drains raw_rx, parses,
        // merges, and emits to the watch sender.
        let (raw_tx, raw_rx) = mpsc::channel::<Vec<u8>>(64);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        // Status worker task. We keep no handle to its mpsc
        // receiver because the worker owns it; the raw_messages_rx
        // field is left None unless something needs out-of-band
        // access to raw payloads.
        let _ = &self.raw_messages_rx;

        let status_tx_for_worker = self.status_tx.clone();
        let worker_task = tokio::spawn(super::status::run_worker(raw_rx, status_tx_for_worker));
        self.tasks.push(worker_task);

        // rumqttc event loop task.
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
        for mut handle in std::mem::take(&mut self.tasks) {
            // Give the task its graceful exit (shutdown channel + raw_tx
            // drop), but never let a wedged task hang disconnect: abort on
            // timeout instead of detaching it (a dropped JoinHandle leaves
            // the task — and its TLS socket — running).
            match tokio::time::timeout(Duration::from_secs(5), &mut handle).await {
                Ok(_) => {}
                Err(_) => {
                    tracing::warn!(driver = %self.id, "bambu task did not exit in 5s; aborting");
                    handle.abort();
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

    async fn send(
        &mut self,
        payload: SendPayload,
        on_progress: UploadProgressFn,
    ) -> Result<SendHandle, DriverError> {
        let (bytes, plate_id, file_basename, use_ams, ams_mapping, ams_mapping2) = match payload {
            SendPayload::Gcode3mf {
                bytes,
                plate_id,
                file_basename,
                use_ams,
                ams_mapping,
                ams_mapping2,
            } => (bytes, plate_id, file_basename, use_ams, ams_mapping, ams_mapping2),
            SendPayload::Gcode { .. } => {
                return Err(DriverError::Other(
                    "BambuDriver only accepts SendPayload::Gcode3mf".into(),
                ));
            }
        };
        let client = self.client.clone().ok_or(DriverError::NotConnected)?;
        let device_id = self.device_id.clone().ok_or(DriverError::NotConnected)?;

        // Project+plate-derived remote name — the printer's
        // recent-files list shows which project/plate this came
        // from. Re-sending the same plate overwrites the prior
        // upload; (project, plate) is unique enough that we don't
        // need a nonce suffix.
        let remote_name = format!("{file_basename}.gcode.3mf");

        // FTPS over suppaftp's tokio-async-native-tls stack — fully async, so a
        // cancelled send just drops this future (the command layer races it
        // against the cancel token) and the connection tears down.
        let mut ftps = super::ftps::connect(&self.config.host, &self.config.access_code).await?;
        let remote_path = super::ftps::upload(&mut ftps, &remote_name, bytes, on_progress).await?;
        // Quit politely; ignore errors — the upload has already landed.
        let _ = ftps.quit().await;

        // Publish the project_file MQTT command. Field shape
        // pinned against a real BBS capture (single-color AMS
        // print on an A1 mini + AMS Lite). Notable:
        //
        //   - `bed_leveling` (American), not `bed_levelling` —
        //     Doridian's doc has the typo; firmware rejects the
        //     British spelling as a missing field.
        //   - `ftp://<remote_path>` URL form for FTPS-uploaded
        //     files; `file:///mnt/sdcard/...` is the generic
        //     "update Bambu Studio" reject path.
        //   - `ams_mapping` is a **real JSON array** whose
        //     length matches the plate's materials list (one
        //     entry per material, filament index `i` ⇔ material
        //     `i + 1`). The earlier "stringified" form was a
        //     misread of OrcaSlicer's calibration path.
        //   - `ams_mapping2` is also required — the firmware
        //     uses both in tandem and silently falls back to
        //     the external spool when only `ams_mapping` is set.
        let sequence_id = self.next_sequence_id();
        let cmd = ProjectFileCommand {
            print: ProjectFileBody {
                sequence_id: &sequence_id,
                command: "project_file",
                param: format!("Metadata/plate_{plate_id}.gcode"),
                project_id: "0",
                profile_id: "0",
                task_id: "0",
                subtask_id: "0",
                subtask_name: &remote_name,
                // `ftp://<name>` — two slashes, no path prefix.
                // File is at the FTPS root (bambu-connect's
                // convention). The `/cache/` directory approach
                // was a `MicroSD R/W exception` rabbit hole; see
                // bambu/ftps.rs docs.
                url: &format!("ftp://{remote_path}"),
                bed_type: "auto",
                timelapse: false,
                bed_leveling: true,
                flow_cali: false,
                vibration_cali: false,
                layer_inspect: false,
                use_ams,
                ams_mapping: &ams_mapping,
                ams_mapping2: &ams_mapping2,
            },
        };
        let body = serde_json::to_vec(&cmd)
            .map_err(|e| DriverError::Other(format!("serialize project_file: {e}")))?;
        let topic = format!("device/{device_id}/request");
        tracing::info!(
            topic = %topic,
            body = %String::from_utf8_lossy(&body),
            "publishing project_file MQTT command",
        );
        client
            .publish(&topic, QoS::AtMostOnce, false, body)
            .await
            .map_err(|e| DriverError::Network(format!("publish project_file: {e}")))?;

        Ok(SendHandle {
            id: sequence_id,
            file_name: remote_name,
        })
    }

    async fn command(&mut self, cmd: PrinterCommand) -> Result<(), DriverError> {
        let client = self.client.clone().ok_or(DriverError::NotConnected)?;
        let device_id = self.device_id.clone().ok_or(DriverError::NotConnected)?;

        // State guards before publishing — invalid transitions
        // return Other without contacting the printer.
        let current_state = self
            .status_tx
            .borrow()
            .job
            .as_ref()
            .map(|j| j.state.clone())
            .unwrap_or(JobState::Idle);
        let (verb, expected) = match cmd {
            PrinterCommand::Pause => {
                if !matches!(current_state, JobState::Printing) {
                    return Err(DriverError::Other(format!(
                        "cannot pause: printer is {current_state:?}, expected Printing"
                    )));
                }
                ("pause", JobState::Paused)
            }
            PrinterCommand::Resume => {
                if !matches!(current_state, JobState::Paused) {
                    return Err(DriverError::Other(format!(
                        "cannot resume: printer is {current_state:?}, expected Paused"
                    )));
                }
                ("resume", JobState::Printing)
            }
            PrinterCommand::Stop => {
                if matches!(current_state, JobState::Idle | JobState::Finished) {
                    return Err(DriverError::Other(format!(
                        "cannot stop: printer is {current_state:?}, no print in progress"
                    )));
                }
                // Stop is acknowledged via Finished or Failed —
                // Bambu firmware reports cancelled prints as FAILED.
                ("stop", JobState::Failed(String::new()))
            }
        };

        let sequence_id = self.next_sequence_id();
        let body = serde_json::to_vec(&CommandRequest {
            print: CommandBody {
                sequence_id: &sequence_id,
                command: verb,
                param: "",
            },
        })
        .map_err(|e| DriverError::Other(format!("serialize {verb}: {e}")))?;

        // OpenBambuAPI documents pause/resume/stop at QoS 1
        // ("higher priority"). Publish + await ack on the status
        // stream.
        let topic = format!("device/{device_id}/request");
        tracing::debug!(
            target: "mqtt",
            serial = %device_id,
            dir = "tx",
            topic = %topic,
            payload = %String::from_utf8_lossy(&body),
            "command",
        );
        client
            .publish(&topic, QoS::AtLeastOnce, false, body)
            .await
            .map_err(|e| DriverError::Network(format!("publish {verb}: {e}")))?;

        // Wait for the state to transition to the expected
        // value. The 10s timeout matches the per-ticket spec.
        let mut rx = self.status_tx.subscribe();
        let deadline = Duration::from_secs(10);
        let wait = async {
            loop {
                if rx.changed().await.is_err() {
                    return Err(DriverError::Other("status stream closed".into()));
                }
                let snap = rx.borrow().clone();
                let new_state = snap
                    .job
                    .as_ref()
                    .map(|j| j.state.clone())
                    .unwrap_or(JobState::Idle);
                if state_satisfies(&new_state, &expected) {
                    return Ok(());
                }
            }
        };
        tokio::time::timeout(deadline, wait)
            .await
            .map_err(|_| DriverError::Protocol(format!("no ack for {verb} within 10s")))?
    }

    async fn set_ams_filament(
        &mut self,
        setting: crate::core::driver::traits::AmsFilamentSetting,
    ) -> Result<(), DriverError> {
        let client = self.client.clone().ok_or(DriverError::NotConnected)?;
        let device_id = self.device_id.clone().ok_or(DriverError::NotConnected)?;

        let sequence_id = self.next_sequence_id();
        let body = serde_json::to_vec(&AmsFilamentSettingRequest {
            print: AmsFilamentSettingBody {
                sequence_id: &sequence_id,
                command: "ams_filament_setting",
                ams_id: setting.ams_id,
                tray_id: setting.tray_id,
                slot_id: setting.slot_id,
                tray_info_idx: &setting.tray_info_idx,
                tray_type: &setting.tray_type,
                tray_sub_brands: &setting.tray_sub_brands,
                tray_color: &setting.tray_color,
                nozzle_temp_min: setting.nozzle_temp_min,
                nozzle_temp_max: setting.nozzle_temp_max,
            },
        })
        .map_err(|e| DriverError::Other(format!("serialize ams_filament_setting: {e}")))?;

        // Fire-and-forget at QoS 1 — unlike pause/resume/stop there's
        // no job-state transition to await; the AMS report reflects the
        // new tray identity on its next push, and the destructive sync
        // mirrors it back into the slot.
        let topic = format!("device/{device_id}/request");
        tracing::debug!(
            target: "mqtt",
            serial = %device_id,
            dir = "tx",
            topic = %topic,
            payload = %String::from_utf8_lossy(&body),
            "ams_filament_setting",
        );
        client
            .publish(&topic, QoS::AtLeastOnce, false, body)
            .await
            .map_err(|e| DriverError::Network(format!("publish ams_filament_setting: {e}")))?;
        Ok(())
    }
}

/// Loose state match — `Failed(_)` collapses to "any failed
/// state" so `Stop` is acknowledged regardless of which failure
/// reason Bambu firmware reports.
fn state_satisfies(actual: &JobState, expected: &JobState) -> bool {
    match (actual, expected) {
        (JobState::Failed(_), JobState::Failed(_)) => true,
        (JobState::Finished, JobState::Failed(_)) | (JobState::Failed(_), JobState::Finished) => {
            // Stop ack can land as either Finished or Failed
            // depending on whether the printer finishes the
            // current layer before stopping.
            true
        }
        (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
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

/// `pause` / `resume` / `stop` MQTT command — shape from
/// OpenBambuAPI `mqtt.md`. Published at QoS 1 per the doc's
/// "higher priority" annotation.
#[derive(Serialize)]
struct CommandRequest<'a> {
    print: CommandBody<'a>,
}

#[derive(Serialize)]
struct CommandBody<'a> {
    sequence_id: &'a str,
    command: &'a str,
    param: &'a str,
}

/// `ams_filament_setting` MQTT command — writes a tray's filament
/// identity. Field shape cross-referenced against Home Assistant's
/// Bambu integration + `bambu-connect`; the AMS fields sit as
/// siblings of `sequence_id`/`command` inside the `print` object.
#[derive(Serialize)]
struct AmsFilamentSettingRequest<'a> {
    print: AmsFilamentSettingBody<'a>,
}

#[derive(Serialize)]
struct AmsFilamentSettingBody<'a> {
    sequence_id: &'a str,
    command: &'a str,
    ams_id: u8,
    tray_id: u8,
    slot_id: u8,
    tray_info_idx: &'a str,
    tray_type: &'a str,
    tray_sub_brands: &'a str,
    tray_color: &'a str,
    nozzle_temp_min: i32,
    nozzle_temp_max: i32,
}

/// `project_file` MQTT command — shape cross-referenced against
/// Home Assistant's Bambu integration + `bambu-connect` (the
/// open-source impls that are known-working against current
/// firmware). The OpenBambuAPI mqtt.md Doridian fork is stale +
/// has a `bed_levelling`/`bed_leveling` spelling typo we got
/// burned by.
#[derive(Serialize)]
struct ProjectFileCommand<'a> {
    print: ProjectFileBody<'a>,
}

#[derive(Serialize)]
struct ProjectFileBody<'a> {
    sequence_id: &'a str,
    command: &'a str,
    /// Path inside the .gcode.3mf zip (Bambu's per-plate gcode
    /// always lives at `Metadata/plate_<N>.gcode`).
    param: String,
    project_id: &'a str,
    profile_id: &'a str,
    task_id: &'a str,
    subtask_id: &'a str,
    subtask_name: &'a str,
    /// URL of the .gcode.3mf on the printer-side filesystem.
    /// For FTPS-uploaded files: `ftp:///cache/<name>` (three
    /// slashes because `<name>` starts with `/cache/`).
    url: &'a str,
    bed_type: &'a str,
    timelapse: bool,
    /// American spelling — firmware treats `bed_levelling`
    /// (British) as a missing required field.
    bed_leveling: bool,
    flow_cali: bool,
    vibration_cali: bool,
    layer_inspect: bool,
    use_ams: bool,
    /// AMS routing array, sized to the plate's materials list
    /// length (filament index `i` ⇔ model material `i + 1`).
    /// Values: 0-based AMS slot id (0..3) when the material is
    /// bound to an AMS-fed slot, `-1` for external spool or
    /// unbound. Real JSON array; firmware rejects the stringified
    /// form.
    ams_mapping: &'a [i8],
    /// Structured `{ams_id, slot_id}` companion to `ams_mapping`,
    /// same length. Required alongside — firmware uses both and
    /// falls back to the external spool when only one is set.
    /// `{255, 0}` = bound to the external spool, `{255, 255}` =
    /// unbound.
    ams_mapping2: &'a [crate::core::slice::pre_slice_gate::AmsMappingV2],
}

/// Background task: own the rumqttc event loop until shutdown.
///
/// On every iteration:
///   - Drain one event (`eventloop.poll()`).
///   - ConnAck → subscribe to the report topic + publish the
///     pushall request. Flip status to Connected.
///   - Publish to report topic → forward bytes to the parser's
///     channel.
///   - Disconnect → flip status to Reconnecting + sleep backoff +
///     retry. (rumqttc's event loop will re-attempt on its
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
    let mut attempt = 0u32;

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
                        attempt = 0;
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
                                tracing::debug!(
                                    target: "mqtt",
                                    serial = %device_id,
                                    dir = "tx",
                                    topic = %request_topic,
                                    payload = %String::from_utf8_lossy(&body),
                                    "pushall",
                                );
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
                        // Full inbound report on the dedicated `mqtt` target
                        // (enable with `RUST_LOG=mqtt=debug`) so the raw
                        // `ams` / `vt_tray` JSON is inspectable when a spool
                        // looks wrong. Logged here, before the bounded
                        // channel can drop it.
                        tracing::debug!(
                            target: "mqtt",
                            serial = %device_id,
                            dir = "rx",
                            topic = %p.topic,
                            len = p.payload.len(),
                            payload = %String::from_utf8_lossy(&p.payload),
                            "report",
                        );
                        // Forward to the parser. If the channel
                        // is full, drop the oldest by closing the
                        // current send and reopening — for now,
                        // just log + drop the message on full.
                        if let Err(e) = raw_tx.try_send(p.payload.to_vec()) {
                            tracing::trace!(error = %e, "raw message channel full or closed");
                        }
                    }
                    Ok(Event::Incoming(Packet::Disconnect)) => {
                        let delay = reconnect_backoff_secs(attempt);
                        attempt = attempt.saturating_add(1);
                        status_tx.send_modify(|s| {
                            s.connection = ConnectionState::Reconnecting {
                                in_seconds: delay as u32,
                                reason: "the printer closed the connection".to_string(),
                            };
                            s.last_updated = std::time::SystemTime::now();
                        });
                        // Race the backoff against shutdown so teardown
                        // interrupts a pending reconnect window instead of
                        // waiting it out (up to 60s).
                        tokio::select! {
                            biased;
                            _ = &mut shutdown_rx => {
                                tracing::debug!(driver_serial = %device_id, "bambu task shutdown during backoff");
                                return;
                            }
                            _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
                        }
                    }
                    Err(e) => {
                        // Network error — rumqttc surfaces these
                        // through the same poll loop; we treat as
                        // transient + backoff. `e` carries the real
                        // cause (e.g. a refused CONNECT on a bad access
                        // code), so thread it into the status.
                        let delay = reconnect_backoff_secs(attempt);
                        attempt = attempt.saturating_add(1);
                        tracing::warn!(error = %e, backoff_secs = delay, "bambu poll error");
                        status_tx.send_modify(|s| {
                            s.connection = ConnectionState::Reconnecting {
                                in_seconds: delay as u32,
                                reason: e.to_string(),
                            };
                            s.last_updated = std::time::SystemTime::now();
                        });
                        // Same as the Disconnect arm: a shutdown during the
                        // backoff window must drop the task promptly.
                        tokio::select! {
                            biased;
                            _ = &mut shutdown_rx => {
                                tracing::debug!(driver_serial = %device_id, "bambu task shutdown during backoff");
                                return;
                            }
                            _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
                        }
                    }
                    _ => {
                        // PingResp / SubAck / etc — uninteresting.
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_constructs_in_disconnected_state() {
        let driver = BambuDriver::new(
            DriverId(1),
            BambuConfig {
                host: "192.0.2.1".into(),
                access_code: "00000000".into(),
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
            },
        );
        assert_eq!(d.id(), DriverId(7));
        assert_eq!(d.kind(), DriverKind::Bambu);
    }

    #[test]
    fn project_file_command_carries_expected_fields() {
        use crate::core::slice::pre_slice_gate::AmsMappingV2;
        let mapping: Vec<i8> = vec![0, 1, 2, 3];
        let mapping2: Vec<AmsMappingV2> = vec![
            AmsMappingV2 {
                ams_id: 0,
                slot_id: 0,
            },
            AmsMappingV2 {
                ams_id: 0,
                slot_id: 1,
            },
            AmsMappingV2 {
                ams_id: 0,
                slot_id: 2,
            },
            AmsMappingV2 {
                ams_id: 0,
                slot_id: 3,
            },
        ];
        let cmd = ProjectFileCommand {
            print: ProjectFileBody {
                sequence_id: "42",
                command: "project_file",
                param: "Metadata/plate_1.gcode".into(),
                project_id: "0",
                profile_id: "0",
                task_id: "0",
                subtask_id: "0",
                subtask_name: "n3o-1-12345.gcode.3mf",
                url: "ftp://n3o-1-12345.gcode.3mf",
                bed_type: "auto",
                timelapse: false,
                bed_leveling: true,
                flow_cali: false,
                vibration_cali: false,
                layer_inspect: false,
                use_ams: true,
                ams_mapping: &mapping,
                ams_mapping2: &mapping2,
            },
        };
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&cmd).unwrap()).unwrap();
        // Pin against a real BBS capture (4-AMS print on A1 mini +
        // AMS Lite, M1..M4 → AMS:1..4).
        let print = &json["print"];
        assert_eq!(print["sequence_id"], "42");
        assert_eq!(print["command"], "project_file");
        assert_eq!(print["param"], "Metadata/plate_1.gcode");
        assert_eq!(print["url"], "ftp://n3o-1-12345.gcode.3mf");
        assert_eq!(print["bed_type"], "auto");
        assert_eq!(print["use_ams"], true);
        assert!(print["use_ams"].is_boolean());
        assert!(print["timelapse"].is_boolean());
        // American spelling — British `bed_levelling` is read as
        // a missing required field.
        assert!(print["bed_leveling"].is_boolean());
        assert!(print.get("bed_levelling").is_none());
        // Earlier (Doridian doc) and current (BBS capture) agree
        // the firmware tolerates absent file + md5.
        assert!(print.get("file").is_none());
        assert!(print.get("md5").is_none());
        // ams_mapping: real JSON array, length = plate's materials
        // list length (one entry per material, indexed by
        // `material - 1`).
        assert_eq!(print["ams_mapping"], serde_json::json!([0, 1, 2, 3]));
        assert!(print["ams_mapping"].is_array());
        // ams_mapping2: structured form, same length.
        assert_eq!(
            print["ams_mapping2"],
            serde_json::json!([
                {"ams_id": 0, "slot_id": 0},
                {"ams_id": 0, "slot_id": 1},
                {"ams_id": 0, "slot_id": 2},
                {"ams_id": 0, "slot_id": 3},
            ])
        );
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

    #[test]
    fn pause_resume_stop_command_shape_matches_doc() {
        // OpenBambuAPI mqtt.md specifies an empty param string
        // for these verbs. Pin both shape + spelling — if either
        // drifts the printer rejects the command.
        for verb in ["pause", "resume", "stop"] {
            let req = CommandRequest {
                print: CommandBody {
                    sequence_id: "0",
                    command: verb,
                    param: "",
                },
            };
            let s = serde_json::to_string(&req).unwrap();
            assert_eq!(
                s,
                format!(
                    "{{\"print\":{{\"sequence_id\":\"0\",\"command\":\"{verb}\",\"param\":\"\"}}}}"
                ),
            );
        }
    }

    #[test]
    fn state_satisfies_collapses_failed_variants() {
        // Stop ack matches any Failed-flavor reason string.
        assert!(state_satisfies(
            &JobState::Failed("user cancelled".into()),
            &JobState::Failed(String::new())
        ));
        // Stop ack also accepts Finished (printer races between
        // "stop now" and "finish current layer first").
        assert!(state_satisfies(
            &JobState::Finished,
            &JobState::Failed(String::new())
        ));
        // Pause is satisfied only by Paused.
        assert!(state_satisfies(&JobState::Paused, &JobState::Paused));
        assert!(!state_satisfies(&JobState::Printing, &JobState::Paused));
    }
}
