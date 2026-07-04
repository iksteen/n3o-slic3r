//! Live camera streams for the Devices view, with a frontend-driven
//! lifecycle and a vendor-neutral source abstraction.
//!
//! The webview opens a stream when the camera panel becomes active and
//! closes it when the panel is hidden — there is no always-on capture.
//! `camera_start` spawns a per-instance worker that drives a
//! [`CameraSource`] and **pushes** each JPEG frame to the frontend over a
//! [`tauri::ipc::Channel`] (raw bytes → an `ArrayBuffer` on the JS side,
//! no base64 tax); `camera_stop` cancels it.
//!
//! ## The source abstraction
//!
//! Printer cameras come in incompatible shapes, so the worker loop knows
//! nothing vendor-specific — it drives a [`CameraSource`]:
//!
//! - **[`setup`](CameraSource::setup)** — one-time work before the first
//!   frame (the Snapmaker U1 wakes its camera daemon over a session-long
//!   mTLS MQTT control plane here; Bambu/Klipper need nothing).
//! - **[`attempt`](CameraSource::attempt)** — one connect-and-stream pass,
//!   forwarding frames to the [`FrameSink`]. Push sources (Bambu's binary
//!   length-prefixed JPEG socket) and poll sources (Moonraker's MJPEG /
//!   `monitor.jpg` HTTP poll) both fit: each just calls `sink.send` per
//!   frame and returns when the consumer goes away or the link fails.
//! - **[`teardown`](CameraSource::teardown)** — cleanup when the worker
//!   stops for good (the U1 releases monitor mode here).
//!
//! Cancellation is **cooperative** (a [`CancellationToken`], not
//! `JoinHandle::abort`) precisely so `teardown` can run its async release
//! before the task exits.
//!
//! Two sources are wired: the Bambu LAN source ([`super::bambu::camera`],
//! a push socket) and the U1 source ([`super::snapmaker::camera`], a
//! Moonraker MJPEG poll behind the mTLS monitor-mode wake). `source_for`
//! returns an error for backends without a camera so the frontend can fall
//! back to its "camera unavailable" state. A new backend slots in as another
//! [`CameraSource`] impl + `source_for` arm without touching the worker,
//! manager, or commands.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;
use tokio_util::sync::CancellationToken;

use super::snapmaker::camera as u1_camera;
use super::snapmaker::camera::SnapMonitorSession;
use super::snapmaker::snap_token::{self, SnapToken};
use super::backoff::reconnect_backoff_secs;
use super::traits::{DriverConfig, DriverError};

/// Where a [`CameraSource`] hands decoded JPEG frames. Wraps the webview
/// channel so sources never touch Tauri's IPC types directly.
pub struct FrameSink {
    channel: Channel<InvokeResponseBody>,
}

impl FrameSink {
    fn new(channel: Channel<InvokeResponseBody>) -> Self {
        Self { channel }
    }

    /// Push one JPEG frame to the webview. Returns `false` once the
    /// channel has been dropped (the panel went away) — sources treat
    /// that as a clean stop signal.
    pub fn send(&self, frame: Vec<u8>) -> bool {
        self.channel.send(InvokeResponseBody::Raw(frame)).is_ok()
    }
}

/// A vendor camera source. The worker loop drives this and owns retry /
/// backoff; impls own the wire protocol and any wake/release.
#[async_trait]
pub trait CameraSource: Send + Sync {
    /// One connect-and-stream pass. Forward each JPEG to `sink`.
    /// `Ok(())` means the sink's consumer went away (stop for good);
    /// `Err` means the link failed and the worker should reconnect.
    async fn attempt(&self, sink: &FrameSink) -> Result<(), DriverError>;

    /// One-time setup before the first [`attempt`](Self::attempt) (e.g.
    /// the U1's mTLS `camera.start_monitor` wake). Default: nothing. A
    /// failing setup should log and return — streaming still proceeds in
    /// case the camera is already awake.
    async fn setup(&self) {}

    /// Cleanup when the worker shuts down for good — consumer gone or a
    /// stop was requested (e.g. the U1's `camera.stop_monitor` release).
    /// Default: nothing.
    async fn teardown(&self) {}
}

/// The Bambu LAN camera: a TLS socket streaming length-prefixed JPEG
/// frames. No wake/release — `setup`/`teardown` stay default no-ops.
struct BambuCameraSource {
    host: String,
    access_code: String,
}

#[async_trait]
impl CameraSource for BambuCameraSource {
    async fn attempt(&self, sink: &FrameSink) -> Result<(), DriverError> {
        super::bambu::camera::stream_once(&self.host, &self.access_code, |frame| sink.send(frame))
            .await
    }
}

/// The Snapmaker U1 camera: a Moonraker JPEG poll kept alive by a
/// session-long mTLS "monitor mode" wake. `setup` opens the mTLS session
/// and starts monitor mode; `attempt` runs the poll loop; `teardown`
/// releases monitor mode. The session is held across the whole view (the
/// daemon only emits while an authorized client stays subscribed), so it
/// lives in an async `Mutex` set by `setup` and taken by `teardown`.
struct U1CameraSource {
    /// Moonraker HTTP host:port (the print/status endpoint, typically :80).
    host: String,
    port: u16,
    /// Paired mTLS material — drives the wake and identifies the device.
    token: SnapToken,
    client: reqwest::Client,
    session: tokio::sync::Mutex<Option<SnapMonitorSession>>,
}

#[async_trait]
impl CameraSource for U1CameraSource {
    async fn setup(&self) {
        let session = u1_camera::wake(&self.token).await;
        *self.session.lock().await = session;
    }

    async fn attempt(&self, sink: &FrameSink) -> Result<(), DriverError> {
        let url = u1_camera::monitor_url(&self.host, self.port);
        let mut last_modified: Option<String> = None;
        loop {
            let outcome = u1_camera::poll_frame(&self.client, &url, last_modified.as_deref()).await?;
            if let Some(value) = outcome.last_modified {
                last_modified = Some(value);
            }
            if let Some(frame) = outcome.frame {
                if !sink.send(frame) {
                    return Ok(()); // consumer gone — clean stop
                }
            }
            tokio::time::sleep(u1_camera::POLL_INTERVAL).await;
        }
    }

    async fn teardown(&self) {
        if let Some(session) = self.session.lock().await.take() {
            session.release().await;
        }
    }
}

/// Build the camera source for an instance + connection config, or an
/// error if the backend has no camera support (a U1 that isn't paired).
fn source_for(
    instance_id: &str,
    config: DriverConfig,
) -> Result<Box<dyn CameraSource>, DriverError> {
    match config {
        DriverConfig::Bambu { host, access_code } => {
            Ok(Box::new(BambuCameraSource { host, access_code }))
        }
        DriverConfig::U1 { host, port } => {
            let token = snap_token::load(instance_id).ok_or_else(|| {
                DriverError::Other(
                    "printer is not paired — pair it in the printer's Connection settings to \
                     enable the camera"
                        .to_owned(),
                )
            })?;
            Ok(Box::new(U1CameraSource {
                host,
                port,
                token,
                client: u1_camera::poll_client()?,
                session: tokio::sync::Mutex::new(None),
            }))
        }
    }
}

/// One running camera worker, tracked by the token that stops it. The task
/// itself runs detached — cooperative cancellation (not `abort`) is what
/// shuts it down, so there is no `JoinHandle` to keep.
struct CameraWorker {
    cancel: CancellationToken,
}

/// Tracks one running camera worker per printer instance.
#[derive(Default)]
pub struct CameraManager {
    workers: Mutex<HashMap<String, CameraWorker>>,
}

impl CameraManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start (or restart) the camera worker for `instance_id`, pushing
    /// frames to `channel`. Replacing an existing worker is intentional:
    /// a fresh panel mount supplies a new channel, so the stale worker —
    /// bound to the dead channel — is cancelled here. Cancellation is
    /// cooperative so the replaced worker still runs its teardown.
    fn start(
        &self,
        instance_id: String,
        source: Box<dyn CameraSource>,
        channel: Channel<InvokeResponseBody>,
    ) {
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let sink = FrameSink::new(channel);
        let id = instance_id.clone();
        // Spawn on Tauri's global async runtime, not `tokio::spawn`: the
        // command this runs from is synchronous, so it executes on the UI
        // thread where no Tokio reactor is in context.
        tauri::async_runtime::spawn(async move {
            run_worker(&id, source.as_ref(), &sink, worker_cancel).await;
        });
        self.install(instance_id, cancel);
    }

    /// Register a worker's cancel token, cancelling any prior worker for
    /// the same instance (a fresh panel mount replaces a stale one). Split
    /// from [`start`](Self::start) so the replace/cancel bookkeeping is
    /// testable without spawning a real worker.
    fn install(&self, instance_id: String, cancel: CancellationToken) {
        if let Some(previous) = self
            .workers
            .lock()
            .expect("camera lock")
            .insert(instance_id, CameraWorker { cancel })
        {
            previous.cancel.cancel();
        }
    }

    /// Stop the worker for `instance_id`, if any. Cooperative — the worker
    /// runs its teardown then exits; we don't block the caller on it.
    /// Idempotent.
    pub fn stop(&self, instance_id: &str) {
        if let Some(worker) = self.workers.lock().expect("camera lock").remove(instance_id) {
            worker.cancel.cancel();
        }
    }
}

/// Drive one [`CameraSource`]: setup once, retry [`attempt`] with backoff
/// until the consumer goes away or cancellation, then teardown once.
async fn run_worker(
    instance_id: &str,
    source: &dyn CameraSource,
    sink: &FrameSink,
    cancel: CancellationToken,
) {
    if cancel.is_cancelled() {
        return;
    }
    source.setup().await;

    let mut attempt: u32 = 0;
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            outcome = source.attempt(sink) => match outcome {
                // Consumer (the webview channel) went away — clean stop.
                Ok(()) => break,
                Err(error) => {
                    tracing::warn!(
                        instance_id,
                        error = %error,
                        "camera stream disconnected; will retry"
                    );
                    let delay = Duration::from_secs(reconnect_backoff_secs(attempt));
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(delay) => {}
                    }
                    attempt = attempt.saturating_add(1);
                }
            },
        }
    }

    source.teardown().await;
}

/// Open a live camera stream for a printer instance. Frames arrive on
/// `channel` as raw-bytes messages (`ArrayBuffer` in JS). Returns an error
/// for backends without camera support so the frontend can show its
/// "camera unavailable" state.
#[tracing::instrument(skip(manager, config, channel))]
#[tauri::command]
pub fn camera_start(
    manager: State<'_, std::sync::Arc<CameraManager>>,
    instance_id: String,
    config: DriverConfig,
    channel: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    // Errors cross the IPC boundary as their Display string (matching every
    // other driver command), so the frontend shows the message rather than
    // a serialized `DriverError` enum.
    let source = source_for(&instance_id, config).map_err(|e| e.to_string())?;
    manager.start(instance_id, source, channel);
    Ok(())
}

/// Close the live camera stream for a printer instance. Idempotent — a
/// stop for an instance with no running worker is a no-op.
#[tracing::instrument(skip(manager))]
#[tauri::command]
pub fn camera_stop(manager: State<'_, std::sync::Arc<CameraManager>>, instance_id: String) {
    manager.stop(&instance_id);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    /// A source that fails every attempt — exercises the retry loop and
    /// the setup/teardown hooks under cooperative cancellation.
    struct FlakySource {
        attempts: Arc<AtomicUsize>,
        setups: Arc<AtomicUsize>,
        teardowns: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl CameraSource for FlakySource {
        async fn attempt(&self, _sink: &FrameSink) -> Result<(), DriverError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            Err(DriverError::Network("nope".into()))
        }
        async fn setup(&self) {
            self.setups.fetch_add(1, Ordering::SeqCst);
        }
        async fn teardown(&self) {
            self.teardowns.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// A source whose first attempt reports the consumer is gone — the
    /// worker must stop on its own (no cancel) and still run teardown.
    struct ConsumerGoneSource {
        teardowns: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl CameraSource for ConsumerGoneSource {
        async fn attempt(&self, _sink: &FrameSink) -> Result<(), DriverError> {
            Ok(())
        }
        async fn teardown(&self) {
            self.teardowns.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn dummy_sink() -> FrameSink {
        // The channel is never driven in these tests (sources ignore it);
        // a Channel needs no live webview to construct.
        FrameSink::new(Channel::new(|_| Ok(())))
    }

    #[tokio::test]
    async fn worker_setups_retries_then_tears_down_on_cancel() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let setups = Arc::new(AtomicUsize::new(0));
        let teardowns = Arc::new(AtomicUsize::new(0));
        let source = FlakySource {
            attempts: Arc::clone(&attempts),
            setups: Arc::clone(&setups),
            teardowns: Arc::clone(&teardowns),
        };
        let cancel = CancellationToken::new();
        let sink = dummy_sink();
        let worker_cancel = cancel.clone();
        let task =
            tokio::spawn(async move { run_worker("t", &source, &sink, worker_cancel).await });

        // The first attempt fails instantly, dropping the worker into its
        // (1s) backoff sleep; cancelling mid-backoff must interrupt it and
        // run teardown rather than hang for the full delay.
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("worker joins promptly on cancel")
            .expect("worker task did not panic");

        assert_eq!(setups.load(Ordering::SeqCst), 1, "setup runs once");
        assert_eq!(teardowns.load(Ordering::SeqCst), 1, "teardown runs once");
        assert!(attempts.load(Ordering::SeqCst) >= 1, "attempted at least once");
    }

    #[tokio::test]
    async fn worker_stops_and_tears_down_when_consumer_gone() {
        let teardowns = Arc::new(AtomicUsize::new(0));
        let source = ConsumerGoneSource {
            teardowns: Arc::clone(&teardowns),
        };
        let sink = dummy_sink();
        // No cancellation — the worker must exit on the Ok(()) stop signal.
        run_worker("t", &source, &sink, CancellationToken::new()).await;
        assert_eq!(teardowns.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn install_cancels_the_previous_worker_for_an_instance() {
        let manager = CameraManager::new();
        let first = CancellationToken::new();
        let second = CancellationToken::new();

        manager.install("printer-a".to_owned(), first.clone());
        assert!(!first.is_cancelled());

        // A fresh mount for the same instance replaces and cancels the old.
        manager.install("printer-a".to_owned(), second.clone());
        assert!(first.is_cancelled(), "replacing cancels the prior worker");
        assert!(!second.is_cancelled());

        // A different instance is independent.
        let other = CancellationToken::new();
        manager.install("printer-b".to_owned(), other.clone());
        assert!(!second.is_cancelled());
        assert!(!other.is_cancelled());

        manager.stop("printer-a");
        assert!(second.is_cancelled());
        assert!(!other.is_cancelled());
    }

    #[test]
    fn stop_is_idempotent_for_unknown_instance() {
        let manager = CameraManager::new();
        manager.stop("never-started");
        manager.stop("never-started");
    }

    #[test]
    fn source_for_rejects_an_unpaired_u1() {
        // No printers root is configured in unit tests, so the U1 has no
        // pairing token — source_for must reject it with pairing guidance.
        let err = source_for(
            "some-instance",
            DriverConfig::U1 {
                host: "h".into(),
                port: 80,
            },
        )
        .err()
        .expect("unpaired U1 has no camera source");
        assert!(err.to_string().contains("not paired"));
    }

    #[test]
    fn source_for_builds_a_bambu_source() {
        assert!(source_for(
            "some-instance",
            DriverConfig::Bambu {
                host: "h".into(),
                access_code: "12345678".into(),
            },
        )
        .is_ok());
    }
}
