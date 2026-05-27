//! Probe a Snapmaker U1 (or any Moonraker) endpoint via
//! `GET /machine/system_info` to learn its stable identity (serial
//! number) and friendly name before the registry locks in a
//! driver instance.
//!
//! Mirrors `infer_local_device_id` on the Bambu side: a startup-
//! time round trip that turns a user-supplied LAN endpoint into a
//! fully-shaped device entry. Driver-register paths that receive
//! a [`crate::core::driver::traits::DriverConfig::U1`] without an
//! explicit `serial` resolve it through here first.
//!
//! Ported from `iksteen/bambu-overlay` `src/snapmaker/probe.rs`.

use std::time::Duration;

use serde::Deserialize;

use crate::core::driver::DriverError;

/// How long to wait for the printer to answer the probe before
/// declaring the network unreachable. Five seconds matches the
/// overlay's choice; a healthy LAN printer answers in <100 ms.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// What `probe_system_info` extracts. Everything else Moonraker
/// returns under `system_info` is ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct U1SystemInfo {
    /// Stable identifier the user-library entry can persist
    /// (`PrinterInstance.connection.serial` when populated). U1
    /// firmware always reports this; an empty string is rejected.
    pub serial: String,
    /// Friendly name from Snapmaker's product info — populated as
    /// `device_name` or, if absent, `machine_type`. `None` when
    /// neither is reported.
    pub name: Option<String>,
}

/// Hit `http://host:port/machine/system_info` and decode the
/// `serial` + name fields. Maps every failure mode into a
/// `DriverError` variant the registry's `register` path already
/// handles: `Network` for socket-level failures, `Protocol` for
/// HTTP errors and unexpected JSON, `Auth` is unused (the
/// endpoint is unauthenticated).
pub async fn probe_system_info(host: &str, port: u16) -> Result<U1SystemInfo, DriverError> {
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
    let body: SystemInfoResponse = response
        .json()
        .await
        .map_err(|e| DriverError::Protocol(format!("Moonraker {url} returned non-JSON: {e}")))?;

    let product = body.result.system_info.product_info;
    let serial = product
        .serial_number
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            DriverError::Protocol(format!(
                "Moonraker {url} did not report a serial_number in machine/system_info",
            ))
        })?;
    let name = product
        .device_name
        .or(product.machine_type)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    Ok(U1SystemInfo { serial, name })
}

#[derive(Deserialize)]
struct SystemInfoResponse {
    result: SystemInfoResult,
}

#[derive(Deserialize)]
struct SystemInfoResult {
    system_info: SystemInfoBody,
}

#[derive(Deserialize)]
struct SystemInfoBody {
    #[serde(default)]
    product_info: ProductInfo,
}

#[derive(Default, Deserialize)]
struct ProductInfo {
    #[serde(default)]
    serial_number: Option<String>,
    #[serde(default)]
    device_name: Option<String>,
    #[serde(default)]
    machine_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body(serial: Option<&str>, device_name: Option<&str>, machine_type: Option<&str>) -> serde_json::Value {
        let mut product = serde_json::Map::new();
        if let Some(s) = serial {
            product.insert("serial_number".into(), s.into());
        }
        if let Some(s) = device_name {
            product.insert("device_name".into(), s.into());
        }
        if let Some(s) = machine_type {
            product.insert("machine_type".into(), s.into());
        }
        serde_json::json!({
            "result": {
                "system_info": {
                    "product_info": product,
                }
            }
        })
    }

    fn host_port(server: &MockServer) -> (String, u16) {
        let uri = server.uri();
        let stripped = uri.strip_prefix("http://").unwrap_or(&uri);
        let (host, port) = stripped.split_once(':').unwrap();
        (host.to_owned(), port.parse().unwrap())
    }

    #[tokio::test]
    async fn happy_path_returns_serial_and_device_name() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/machine/system_info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body(
                Some("SN-U1-12345"),
                Some("Garage U1"),
                Some("Snapmaker U1"),
            )))
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        let info = probe_system_info(&host, port).await.unwrap();
        assert_eq!(info.serial, "SN-U1-12345");
        // device_name wins over machine_type when both present.
        assert_eq!(info.name.as_deref(), Some("Garage U1"));
    }

    #[tokio::test]
    async fn name_falls_back_to_machine_type_when_device_name_absent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/machine/system_info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body(
                Some("SN-U1-12345"),
                None,
                Some("Snapmaker U1"),
            )))
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        let info = probe_system_info(&host, port).await.unwrap();
        assert_eq!(info.name.as_deref(), Some("Snapmaker U1"));
    }

    #[tokio::test]
    async fn name_is_none_when_neither_field_set() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/machine/system_info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body(
                Some("SN-U1-12345"),
                None,
                None,
            )))
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        let info = probe_system_info(&host, port).await.unwrap();
        assert!(info.name.is_none());
    }

    #[tokio::test]
    async fn missing_serial_is_a_protocol_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/machine/system_info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body(
                None,
                Some("Garage U1"),
                None,
            )))
            .mount(&server)
            .await;
        let (host, port) = host_port(&server);
        let err = probe_system_info(&host, port).await.unwrap_err();
        assert!(matches!(err, DriverError::Protocol(_)), "{err:?}");
        assert!(err.to_string().contains("serial_number"));
    }

    #[tokio::test]
    async fn empty_serial_is_a_protocol_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/machine/system_info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body(
                Some("   "),
                None,
                None,
            )))
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
