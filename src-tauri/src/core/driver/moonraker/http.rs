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

use crate::core::driver::traits::{
    PrinterCommand, SendHandle, U1StartOptions, UploadProgressFn,
};
use crate::core::driver::DriverError;

/// Stream the upload body in chunks this size, reporting progress per chunk —
/// small enough for smooth feedback, large enough not to add overhead.
const UPLOAD_CHUNK: usize = 64 * 1024;

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

/// Upload the G-code body + start the print. Returns a [`SendHandle`]
/// whose `id` is the printer-visible filename so subsequent status
/// snapshots (`print_stats.filename`) correlate cleanly.
///
/// The start depends on `u1_start`:
/// - `None` (generic Moonraker) — `print=true` in the upload itself, so
///   the file can't land on disk without a start.
/// - `Some` (Snapmaker U1) — upload only, then start via the vendor
///   `SDCARD_PRINT_FILE_WITH_PARAMETERS` macro so the per-print toggles
///   (leveling / flow / shaper calibration, timelapse) ride the start.
///   The plain `print=true` path would run the macro-less
///   `SDCARD_PRINT_FILE`, which leaves the printer's persisted
///   `print_task_config` untouched.
pub(super) async fn upload_and_start(
    host: &str,
    port: u16,
    file_name: &str,
    bytes: Vec<u8>,
    u1_start: Option<&U1StartOptions>,
    on_progress: UploadProgressFn,
) -> Result<SendHandle, DriverError> {
    let url = format!("http://{host}:{port}/server/files/upload");
    // Stream the body in chunks so reqwest reports real upload progress as it
    // drains them to the socket — `Part::bytes` would buffer the whole body and
    // give no signal. The source `bytes` is shared (Arc) and sliced one chunk at
    // a time (64 KiB copy), so peak memory stays flat. `on_progress(sent, total)`
    // fires per chunk as the stream is polled.
    let total = bytes.len() as u64;
    let buf = std::sync::Arc::new(bytes);
    let n_chunks = buf.len().div_ceil(UPLOAD_CHUNK);
    let body_stream = futures_util::stream::iter((0..n_chunks).map(move |i| {
        let start = i * UPLOAD_CHUNK;
        let end = (start + UPLOAD_CHUNK).min(buf.len());
        on_progress(end as u64, total);
        Ok::<Vec<u8>, std::io::Error>(buf[start..end].to_vec())
    }));
    // Moonraker's upload endpoint keys on the `file` form field's
    // filename header — that becomes `print_stats.filename` in
    // subsequent status events. We thread `file_name` through
    // verbatim so the caller controls the printer-side identity.
    let file_part = Part::stream_with_length(reqwest::Body::wrap_stream(body_stream), total)
        .file_name(file_name.to_owned())
        .mime_str("application/octet-stream")
        .map_err(|e| DriverError::Other(format!("multipart body build: {e}")))?;
    let mut form = Form::new().part("file", file_part);
    if u1_start.is_none() {
        // `print=true` queues the print as soon as the file lands.
        // Without it the upload succeeds but the print never starts,
        // which is exactly the surprise we want to avoid. The U1 path
        // starts via its parameterized macro right below instead.
        form = form.text("print", "true");
    }
    let response = client()?
        .post(&url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| DriverError::Network(format!("upload to {url}: {e}")))?;
    response
        .error_for_status()
        .map_err(|e| DriverError::Protocol(format!("Moonraker upload at {url}: {e}")))?;

    if let Some(start) = u1_start {
        run_gcode_script(host, port, &u1_start_script(file_name, start)).await?;
    }

    Ok(SendHandle {
        id: file_name.to_owned(),
        file_name: file_name.to_owned(),
    })
}

/// Build the U1's parameterized start command. `BED_LEVEL` /
/// `FLOW_CALIBRATE` / `SHAPER_CALIBRATE` / `TIME_LAPSE_CAMERA` are 0/1
/// toggles; flow calibration is additionally gated by
/// `FLOW_CALIBRATE_EXTRUDERS` (the physical extruders to calibrate) and
/// `FILAMENT_USED_MM` (per-extruder usage), which the firmware parses as
/// bracketed lists. Verified against `Snapmaker/u1-klipper`'s
/// `print_task_config.py::cmd_SET_PRINT_TASK_PARAMETERS`.
fn u1_start_script(file_name: &str, start: &U1StartOptions) -> String {
    let o = &start.options;
    let mut script = format!(
        "SDCARD_PRINT_FILE_WITH_PARAMETERS FILENAME=\"{file_name}\" \
         BED_LEVEL={} FLOW_CALIBRATE={} SHAPER_CALIBRATE={} TIME_LAPSE_CAMERA={}",
        u8::from(o.bed_leveling),
        u8::from(o.flow_calibration),
        u8::from(o.vibration_calibration),
        u8::from(o.timelapse),
    );
    if !start.extruders_used.is_empty() {
        let list = start
            .extruders_used
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(",");
        script.push_str(&format!(" FLOW_CALIBRATE_EXTRUDERS=\"[{list}]\""));
    }
    if !start.filament_used_mm.is_empty() {
        let list = start
            .filament_used_mm
            .iter()
            .map(|v| format!("{v:.1}"))
            .collect::<Vec<_>>()
            .join(",");
        script.push_str(&format!(" FILAMENT_USED_MM=\"[{list}]\""));
    }
    if !start.nozzle_diameters.is_empty() {
        let list = start
            .nozzle_diameters
            .iter()
            .map(|v| float_literal(*v))
            .collect::<Vec<_>>()
            .join(",");
        script.push_str(&format!(" NOZZLE_DIAMETER_LIST=\"[{list}]\""));
    }
    if !start.map_table.is_empty() {
        let pairs = start
            .map_table
            .iter()
            .map(|(logical, physical)| format!("[{logical},{physical}]"))
            .collect::<Vec<_>>()
            .join(",");
        script.push_str(&format!(" MAP_TABLE=\"[{pairs}]\""));
    }
    script
}

/// Format a float so the firmware's list parser reads it back as a
/// float: entries without a decimal point parse as ints and are
/// rejected (`isinstance(x, float)` checks in `print_task_config.py`).
fn float_literal(v: f64) -> String {
    let s = v.to_string();
    if s.contains('.') {
        s
    } else {
        format!("{s}.0")
    }
}

/// POST a G-code script to `/printer/gcode/script`. The U1 start macro
/// rides this; Klipper errors (e.g. "SD busy", "not allow to set
/// parameters during printing") come back as 4xx and surface as
/// `DriverError::Protocol`.
async fn run_gcode_script(host: &str, port: u16, script: &str) -> Result<(), DriverError> {
    let url = format!("http://{host}:{port}/printer/gcode/script");
    let response = client()?
        .post(&url)
        .json(&serde_json::json!({ "script": script }))
        .send()
        .await
        .map_err(|e| DriverError::Network(format!("POST {url}: {e}")))?;
    response
        .error_for_status()
        .map_err(|e| DriverError::Protocol(format!("Moonraker gcode script at {url}: {e}")))?;
    Ok(())
}

/// POST `/printer/print/{action}`. Moonraker maps these directly to
/// Klipper's PAUSE / RESUME / CANCEL_PRINT g-code macros.
///
/// Klipper rejects illegal transitions (pause from idle, resume from
/// printing, cancel without an active print) with a 4xx response;
/// `error_for_status()` lifts that into `DriverError::Protocol` so
/// the caller can show "no active print to pause" without contacting
/// the driver again.
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

    // ---- U1 parameterized start ----

    #[test]
    fn u1_start_script_encodes_toggles_and_flow_gating() {
        use crate::core::driver::traits::SendOptions;
        let start = U1StartOptions {
            options: SendOptions {
                bed_leveling: true,
                flow_calibration: true,
                vibration_calibration: false,
                timelapse: true,
            },
            extruders_used: vec![0, 1],
            filament_used_mm: vec![500.0, 600.6],
            nozzle_diameters: vec![0.4, 0.4, 0.6, 0.4],
            map_table: vec![(0, 0), (1, 1), (2, 2), (3, 3)],
        };
        assert_eq!(
            u1_start_script("MyPrint_Lid.gcode", &start),
            "SDCARD_PRINT_FILE_WITH_PARAMETERS FILENAME=\"MyPrint_Lid.gcode\" \
             BED_LEVEL=1 FLOW_CALIBRATE=1 SHAPER_CALIBRATE=0 TIME_LAPSE_CAMERA=1 \
             FLOW_CALIBRATE_EXTRUDERS=\"[0,1]\" FILAMENT_USED_MM=\"[500.0,600.6]\" \
             NOZZLE_DIAMETER_LIST=\"[0.4,0.4,0.6,0.4]\" \
             MAP_TABLE=\"[[0,0],[1,1],[2,2],[3,3]]\""
        );
    }

    #[test]
    fn u1_start_script_encodes_a_cross_mapping() {
        // Non-identity table: material 0 → toolhead 2, material 1 → 0.
        let start = U1StartOptions {
            options: Default::default(),
            extruders_used: vec![],
            filament_used_mm: vec![],
            nozzle_diameters: vec![],
            map_table: vec![(0, 2), (1, 0), (2, 2), (3, 3)],
        };
        let script = u1_start_script("x.gcode", &start);
        assert!(
            script.ends_with(" MAP_TABLE=\"[[0,2],[1,0],[2,2],[3,3]]\""),
            "{script}"
        );
    }

    #[test]
    fn u1_start_script_omits_flow_arrays_without_usage_data() {
        // A G-code without usage lines yields empty arrays — the script
        // must omit the params so the firmware keeps its persisted values
        // rather than erroring on an empty list.
        let start = U1StartOptions {
            options: Default::default(),
            extruders_used: vec![],
            filament_used_mm: vec![],
            nozzle_diameters: vec![],
            map_table: vec![],
        };
        let script = u1_start_script("x.gcode", &start);
        assert_eq!(
            script,
            "SDCARD_PRINT_FILE_WITH_PARAMETERS FILENAME=\"x.gcode\" \
             BED_LEVEL=1 FLOW_CALIBRATE=0 SHAPER_CALIBRATE=0 TIME_LAPSE_CAMERA=0"
        );
        assert!(!script.contains("FLOW_CALIBRATE_EXTRUDERS"));
        assert!(!script.contains("FILAMENT_USED_MM"));
        assert!(!script.contains("MAP_TABLE"));
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
        // Capture progress: a 4-byte body is one chunk, so the callback fires
        // once and reaches (total, total).
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(u64, u64)>::new()));
        let seen_cb = seen.clone();
        let handle = upload_and_start(
            &host,
            port,
            "Cube.gcode",
            b"G28\n".to_vec(),
            None,
            std::sync::Arc::new(move |sent, total| seen_cb.lock().unwrap().push((sent, total))),
        )
        .await
        .unwrap();
        assert_eq!(handle.id, "Cube.gcode");
        assert_eq!(handle.file_name, "Cube.gcode");
        assert_eq!(
            seen.lock().unwrap().last().copied(),
            Some((4, 4)),
            "upload progress fires and reaches total",
        );
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
        let err = upload_and_start(
            &host,
            port,
            "x.gcode",
            b"G28\n".to_vec(),
            None,
            std::sync::Arc::new(|_, _| {}),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DriverError::Protocol(_)), "{err:?}");
    }

    #[tokio::test]
    async fn upload_unreachable_host_maps_to_network_error() {
        // Reserved port — nothing should be listening.
        let err = upload_and_start("127.0.0.1", 1, "x.gcode", vec![], None, std::sync::Arc::new(|_, _| {}))
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
