//! Generic Moonraker webcam support: discovery via
//! `GET /server/webcams/list` plus the JPEG snapshot poll shared by
//! every Moonraker-served camera (the U1's monitor-mode camera polls
//! the same way; only its wake dance is vendor-specific).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{CONTENT_TYPE, IF_MODIFIED_SINCE, LAST_MODIFIED};
use serde::Deserialize;

use crate::core::driver::traits::DriverError;

const POLL_TIMEOUT: Duration = Duration::from_secs(8);
/// How often to re-poll the snapshot. Matches the cadence Moonraker
/// front-ends (and the U1 daemon) refresh at — a few frames a second.
pub const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// One entry of Moonraker's `/server/webcams/list` response. Only the
/// fields the camera source consumes; everything else in the payload
/// (service, flip flags, FPS hints) is ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct Webcam {
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub snapshot_url: String,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct WebcamListResult {
    webcams: Vec<Webcam>,
}

#[derive(Deserialize)]
struct WebcamListResponse {
    result: WebcamListResult,
}

/// Fetch the printer's configured webcams. An empty list is a valid
/// answer (no webcams configured), distinct from a transport error.
pub async fn list_webcams(
    client: &reqwest::Client,
    host: &str,
    port: u16,
) -> Result<Vec<Webcam>, DriverError> {
    let url = format!("http://{host}:{port}/server/webcams/list");
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| DriverError::Network(format!("webcams list: {e}")))?;
    if !response.status().is_success() {
        return Err(DriverError::Protocol(format!(
            "webcams list returned {}",
            response.status()
        )));
    }
    let parsed: WebcamListResponse = response
        .json()
        .await
        .map_err(|e| DriverError::Protocol(format!("decode webcams list: {e}")))?;
    Ok(parsed.result.webcams)
}

/// Pick the webcam to stream: the first enabled entry with a snapshot
/// URL. `None` when the printer has no usable webcam configured.
// ponytail: snapshot poll only — an MJPEG `stream_url` reader
// (multipart/x-mixed-replace) is the upgrade path if 4 fps ever
// feels too coarse.
pub fn pick_webcam(webcams: &[Webcam]) -> Option<&Webcam> {
    webcams
        .iter()
        .find(|w| w.enabled && !w.snapshot_url.trim().is_empty())
}

/// Resolve a webcam URL against the printer host. Moonraker stores
/// them relative (`/webcam/?action=snapshot`) or absolute; relative
/// ones resolve against the plain-HTTP Moonraker endpoint.
pub fn resolve_url(host: &str, port: u16, url: &str) -> String {
    let url = url.trim();
    if url.starts_with("http://") || url.starts_with("https://") {
        url.to_owned()
    } else {
        format!("http://{host}:{port}/{}", url.trim_start_matches('/'))
    }
}

/// One JPEG poll. Returns the new frame (when the source changed) plus
/// the `Last-Modified` to feed the next request's `If-Modified-Since`.
pub struct PollOutcome {
    pub frame: Option<Vec<u8>>,
    pub last_modified: Option<String>,
}

pub async fn poll_frame(
    client: &reqwest::Client,
    url: &str,
    last_modified: Option<&str>,
) -> Result<PollOutcome, DriverError> {
    // Cache-bust each request — intermediate caches otherwise replay the
    // same JPEG even after the source writes a new one.
    let nocache = unix_millis_id();
    let sep = if url.contains('?') { '&' } else { '?' };
    let mut request = client.get(format!("{url}{sep}_nocache={nocache}"));
    if let Some(value) = last_modified {
        request = request.header(IF_MODIFIED_SINCE, value);
    }
    let response = request
        .send()
        .await
        .map_err(|e| DriverError::Network(format!("webcam poll: {e}")))?;

    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(PollOutcome {
            frame: None,
            last_modified: None,
        });
    }
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let new_last_modified = response
        .headers()
        .get(LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = response
        .bytes()
        .await
        .map_err(|e| DriverError::Network(format!("read webcam body: {e}")))?;

    if bytes.starts_with(&[0xff, 0xd8]) {
        Ok(PollOutcome {
            frame: Some(bytes.to_vec()),
            last_modified: new_last_modified,
        })
    } else {
        Err(DriverError::Protocol(format!(
            "webcam response is not a JPEG: status={status} content_type={content_type:?} bytes={n}",
            n = bytes.len()
        )))
    }
}

/// A reqwest client tuned for the poll loop.
pub fn poll_client() -> Result<reqwest::Client, DriverError> {
    reqwest::Client::builder()
        .timeout(POLL_TIMEOUT)
        .build()
        .map_err(|e| DriverError::Other(format!("build webcam HTTP client: {e}")))
}

fn unix_millis_id() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(name: &str, enabled: bool, snapshot: &str) -> Webcam {
        Webcam {
            name: name.into(),
            enabled,
            snapshot_url: snapshot.into(),
        }
    }

    #[test]
    fn pick_prefers_first_enabled_with_snapshot() {
        let cams = vec![
            cam("disabled", false, "/webcam/?action=snapshot"),
            cam("streamless", true, ""),
            cam("good", true, "/webcam2/?action=snapshot"),
        ];
        assert_eq!(pick_webcam(&cams).unwrap().name, "good");
        assert!(pick_webcam(&[]).is_none());
    }

    #[test]
    fn resolve_url_handles_relative_and_absolute() {
        assert_eq!(
            resolve_url("printer.local", 80, "/webcam/?action=snapshot"),
            "http://printer.local:80/webcam/?action=snapshot"
        );
        assert_eq!(
            resolve_url("printer.local", 80, "webcam/?action=snapshot"),
            "http://printer.local:80/webcam/?action=snapshot"
        );
        assert_eq!(
            resolve_url("printer.local", 80, "http://other:8080/snap"),
            "http://other:8080/snap"
        );
    }

    #[test]
    fn webcam_list_decodes_moonraker_shape() {
        let body = serde_json::json!({
            "result": { "webcams": [
                { "name": "cam", "enabled": true,
                  "snapshot_url": "/webcam/?action=snapshot",
                  "stream_url": "/webcam/?action=stream",
                  "service": "mjpegstreamer-adaptive" },
                // Sparse entry — defaults apply (enabled, empty snapshot).
                { "name": "bare" },
            ]}
        });
        let parsed: WebcamListResponse = serde_json::from_value(body).unwrap();
        let cams = parsed.result.webcams;
        assert_eq!(cams.len(), 2);
        assert_eq!(cams[0].snapshot_url, "/webcam/?action=snapshot");
        assert!(cams[1].enabled, "enabled defaults to true");
        assert!(cams[1].snapshot_url.is_empty());
    }
}
