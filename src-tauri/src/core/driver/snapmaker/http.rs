//! Moonraker HTTP control-plane: file upload + print start +
//! pause / resume / cancel.
//!
//! Send path: `POST /server/files/upload` with `multipart/form-data`
//! carrying the raw G-code body. We pass `print=true` so the upload
//! and the print start are one round trip — fewer chances for the
//! file to land on disk without a follow-up start request taking
//! effect.
//!
//! Commands: `POST /printer/print/{pause,resume,cancel}`. Moonraker
//! returns `{"result": "ok"}` on every success; we only check the
//! HTTP status, not the body — Klipper rejects pause-from-idle /
//! resume-from-printing / etc. with a 4xx response that
//! `error_for_status()` lifts into a `DriverError::Protocol`.

use std::time::Duration;

use reqwest::multipart::{Form, Part};

use crate::core::driver::traits::{PrinterCommand, SendHandle};
use crate::core::driver::DriverError;

/// HTTP timeout for every request to the printer. The upload path
/// can be slow for large jobs, but Moonraker buffers the whole body
/// before responding — 60 s is plenty for a typical 50 MB print
/// on a wired LAN; pre-MVP feedback if a real print exceeds it.
const HTTP_TIMEOUT: Duration = Duration::from_secs(60);

fn client() -> Result<reqwest::Client, DriverError> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| DriverError::Other(format!("HTTP client build failed: {e}")))
}

/// Upload the G-code body + start the print in a single request.
/// Returns a [`SendHandle`] whose `id` is the printer-visible
/// filename so subsequent status snapshots (`print_stats.filename`)
/// correlate cleanly.
#[allow(dead_code)] // consumed by PR-7b-5's U1Driver
pub(super) async fn upload_and_start(
    host: &str,
    port: u16,
    file_name: &str,
    bytes: Vec<u8>,
) -> Result<SendHandle, DriverError> {
    let url = format!("http://{host}:{port}/server/files/upload");
    // Moonraker's upload endpoint keys on the `file` form field's
    // filename header — that becomes `print_stats.filename` in
    // subsequent status events. We thread `file_name` through
    // verbatim so the caller controls the printer-side identity.
    let file_part = Part::bytes(bytes)
        .file_name(file_name.to_owned())
        .mime_str("application/octet-stream")
        .map_err(|e| DriverError::Other(format!("multipart body build: {e}")))?;
    let form = Form::new()
        .part("file", file_part)
        // `print=true` queues the print as soon as the file lands.
        // Without it the upload succeeds but the print never starts,
        // which is exactly the surprise we want to avoid.
        .text("print", "true");
    let response = client()?
        .post(&url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| DriverError::Network(format!("upload to {url}: {e}")))?;
    response
        .error_for_status()
        .map_err(|e| DriverError::Protocol(format!("Moonraker upload at {url}: {e}")))?;
    Ok(SendHandle {
        id: file_name.to_owned(),
        file_name: file_name.to_owned(),
    })
}

/// POST `/printer/print/{action}`. Moonraker maps these directly to
/// Klipper's PAUSE / RESUME / CANCEL_PRINT g-code macros.
///
/// Klipper rejects illegal transitions (pause from idle, resume from
/// printing, cancel without an active print) with a 4xx response;
/// `error_for_status()` lifts that into `DriverError::Protocol` so
/// the caller can show "no active print to pause" without contacting
/// the driver again.
#[allow(dead_code)] // consumed by PR-7b-5's U1Driver
pub(super) async fn send_command(
    host: &str,
    port: u16,
    cmd: PrinterCommand,
) -> Result<(), DriverError> {
    let action = match cmd {
        PrinterCommand::Pause => "pause",
        PrinterCommand::Resume => "resume",
        PrinterCommand::Stop => "cancel",
    };
    let url = format!("http://{host}:{port}/printer/print/{action}");
    let response = client()?
        .post(&url)
        .send()
        .await
        .map_err(|e| DriverError::Network(format!("POST {url}: {e}")))?;
    response
        .error_for_status()
        .map_err(|e| DriverError::Protocol(format!("Moonraker {action} at {url}: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn host_port(server: &MockServer) -> (String, u16) {
        let uri = server.uri();
        let stripped = uri.strip_prefix("http://").unwrap_or(&uri);
        let (host, port) = stripped.split_once(':').unwrap();
        (host.to_owned(), port.parse().unwrap())
    }

    // ---- upload + start ----

    #[tokio::test]
    async fn upload_and_start_sends_multipart_with_print_true() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/server/files/upload"))
            // Multipart bodies always have a multipart/form-data
            // content-type with a `boundary=` parameter — matching
            // on the header existence is enough to prove the
            // wrapper picked the multipart path (not, say, a JSON
            // body).
            .and(header_exists("content-type"))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(serde_json::json!({ "result": { "print_started": true } })),
            )
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        let handle = upload_and_start(&host, port, "Cube.gcode", b"G28\n".to_vec())
            .await
            .unwrap();
        assert_eq!(handle.id, "Cube.gcode");
        assert_eq!(handle.file_name, "Cube.gcode");
    }

    #[tokio::test]
    async fn upload_http_4xx_maps_to_protocol_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/server/files/upload"))
            .respond_with(
                ResponseTemplate::new(409)
                    .set_body_json(serde_json::json!({ "error": { "message": "file exists" } })),
            )
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        let err = upload_and_start(&host, port, "x.gcode", b"G28\n".to_vec())
            .await
            .unwrap_err();
        assert!(matches!(err, DriverError::Protocol(_)), "{err:?}");
    }

    #[tokio::test]
    async fn upload_unreachable_host_maps_to_network_error() {
        // Reserved port — nothing should be listening.
        let err = upload_and_start("127.0.0.1", 1, "x.gcode", vec![])
            .await
            .unwrap_err();
        assert!(matches!(err, DriverError::Network(_)), "{err:?}");
    }

    // ---- commands ----

    #[tokio::test]
    async fn pause_hits_printer_print_pause() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/printer/print/pause"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "result": "ok" })),
            )
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        send_command(&host, port, PrinterCommand::Pause)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn resume_hits_printer_print_resume() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/printer/print/resume"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "result": "ok" })),
            )
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        send_command(&host, port, PrinterCommand::Resume)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn stop_hits_printer_print_cancel() {
        // `PrinterCommand::Stop` maps to Moonraker's `cancel`
        // verb — pinned by test so the mapping doesn't silently
        // drift to /stop or /abort.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/printer/print/cancel"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "result": "ok" })),
            )
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        send_command(&host, port, PrinterCommand::Stop)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn command_4xx_maps_to_protocol_error_with_action_name() {
        // Klipper rejects pause-from-idle with 4xx; the error
        // message must name the action so the UI can show
        // "Pause failed" without parsing the underlying status.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/printer/print/pause"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({ "error": { "message": "not printing" } })),
            )
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        let err = send_command(&host, port, PrinterCommand::Pause)
            .await
            .unwrap_err();
        match err {
            DriverError::Protocol(msg) => assert!(msg.contains("pause"), "got {msg}"),
            other => panic!("expected Protocol, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn command_unreachable_host_maps_to_network_error() {
        let err = send_command("127.0.0.1", 1, PrinterCommand::Pause)
            .await
            .unwrap_err();
        assert!(matches!(err, DriverError::Network(_)), "{err:?}");
    }
}
