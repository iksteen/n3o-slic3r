//! Probe a Moonraker endpoint via `GET /machine/system_info` as the
//! connect-time reachability + identity check: a decodable
//! `result.system_info` proves the host speaks Moonraker before the
//! driver spawns its worker, turning a wrong host into a clean
//! immediate error instead of a stuck "connecting…".
//!
//! Ported from `iksteen/bambu-overlay` `src/snapmaker/probe.rs`,
//! minus the vendor product-identity extraction — nothing consumed
//! it, and vanilla Moonraker (Mainsail OS, Fluidd) reports no
//! `product_info` at all.

use std::time::Duration;

use serde::Deserialize;

use crate::core::driver::DriverError;

/// How long to wait for the printer to answer the probe before
/// declaring the network unreachable. Five seconds matches the
/// overlay's choice; a healthy LAN printer answers in <100 ms.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Hit `http://host:port/machine/system_info` and verify the response
/// decodes as a Moonraker `system_info` report. Maps every failure
/// mode into a `DriverError` variant the registry's `register` path
/// already handles: `Network` for socket-level failures, `Protocol`
/// for HTTP errors and unexpected JSON, `Auth` is unused (the
/// endpoint is unauthenticated).
pub async fn probe_system_info(host: &str, port: u16) -> Result<(), DriverError> {
    let url = format!("http://{host}:{port}/machine/system_info");
    let client = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
        .map_err(|e| DriverError::Other(format!("HTTP client build failed: {e}")))?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| DriverError::Network(format!("GET {url}: {e}")))?
        .error_for_status()
        .map_err(|e| DriverError::Protocol(format!("Moonraker {url} returned an error: {e}")))?;
    // Decode (and discard) the envelope — a missing `result.system_info`
    // means the host answered HTTP but isn't Moonraker.
    let _body: SystemInfoResponse = response
        .json()
        .await
        .map_err(|e| DriverError::Protocol(format!("Moonraker {url} returned non-JSON: {e}")))?;
    Ok(())
}

/// Read the active extruder's applied `pressure_advance` via
/// `GET /printer/objects/query?extruder`. Used after `FLOW_CALIBRATE`
/// (which applies the measured K to the active toolhead) to read the
/// result back — n3o doesn't capture gcode-response text, and the
/// extruder object carries the live value. `Protocol` error if the
/// response has no numeric `pressure_advance`.
pub async fn query_pressure_advance(host: &str, port: u16) -> Result<f64, DriverError> {
    let url = format!("http://{host}:{port}/printer/objects/query?extruder");
    let client = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
        .map_err(|e| DriverError::Other(format!("HTTP client build failed: {e}")))?;
    let body: ExtruderQueryResponse = client
        .get(&url)
        .send()
        .await
        .map_err(|e| DriverError::Network(format!("GET {url}: {e}")))?
        .error_for_status()
        .map_err(|e| DriverError::Protocol(format!("Moonraker {url} returned an error: {e}")))?
        .json()
        .await
        .map_err(|e| DriverError::Protocol(format!("Moonraker {url} returned non-JSON: {e}")))?;
    body.result
        .status
        .extruder
        .pressure_advance
        .ok_or_else(|| DriverError::Protocol("extruder status carried no pressure_advance".into()))
}

#[derive(Deserialize)]
struct ExtruderQueryResponse {
    result: ExtruderQueryResult,
}

#[derive(Deserialize)]
struct ExtruderQueryResult {
    status: ExtruderQueryStatus,
}

#[derive(Deserialize)]
struct ExtruderQueryStatus {
    extruder: ExtruderObj,
}

#[derive(Deserialize)]
struct ExtruderObj {
    pressure_advance: Option<f64>,
}

#[derive(Deserialize)]
struct SystemInfoResponse {
    #[allow(dead_code)]
    result: SystemInfoResult,
}

#[derive(Deserialize)]
struct SystemInfoResult {
    #[allow(dead_code)]
    system_info: serde_json::Map<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn host_port(server: &MockServer) -> (String, u16) {
        let uri = server.uri();
        let stripped = uri.strip_prefix("http://").unwrap_or(&uri);
        let (host, port) = stripped.split_once(':').unwrap();
        (host.to_owned(), port.parse().unwrap())
    }

    #[tokio::test]
    async fn vanilla_moonraker_system_info_probes_ok() {
        // Mainsail OS / Fluidd stock Moonraker: `system_info` carries
        // cpu_info/distribution/etc — no vendor product_info. The
        // probe only cares that the envelope decodes.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/machine/system_info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "system_info": {
                    "cpu_info": { "cpu_count": 4 },
                    "distribution": { "name": "MainsailOS" },
                }}
            })))
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        probe_system_info(&host, port).await.unwrap();
    }

    #[tokio::test]
    async fn snapmaker_system_info_with_product_info_probes_ok() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/machine/system_info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "system_info": { "product_info": {
                    "serial_number": "SN-U1-12345",
                    "device_name": "Garage U1",
                }}}
            })))
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        probe_system_info(&host, port).await.unwrap();
    }

    #[tokio::test]
    async fn missing_system_info_is_a_protocol_error() {
        // An HTTP server that answers 200 with the wrong shape (not
        // Moonraker) must fail the probe, not pass as reachable.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/machine/system_info"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})),
            )
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        let err = probe_system_info(&host, port).await.unwrap_err();
        assert!(matches!(err, DriverError::Protocol(_)), "{err:?}");
    }

    #[tokio::test]
    async fn http_500_maps_to_protocol_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/machine/system_info"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        let err = probe_system_info(&host, port).await.unwrap_err();
        assert!(matches!(err, DriverError::Protocol(_)), "{err:?}");
    }

    #[tokio::test]
    async fn non_json_body_maps_to_protocol_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/machine/system_info"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        let err = probe_system_info(&host, port).await.unwrap_err();
        assert!(matches!(err, DriverError::Protocol(_)), "{err:?}");
    }

    #[tokio::test]
    async fn unreachable_host_maps_to_network_error() {
        // Port 1 is reserved + nothing should be listening on it.
        let err = probe_system_info("127.0.0.1", 1).await.unwrap_err();
        assert!(matches!(err, DriverError::Network(_)), "{err:?}");
    }
}
