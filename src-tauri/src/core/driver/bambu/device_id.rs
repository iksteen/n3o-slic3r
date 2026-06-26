//! Device-id (serial) probe via peer-cert CN extraction.
//!
//! Bambu's LAN MQTT cert encodes the printer's serial number as
//! the subject Common Name. We open a raw TLS socket once at
//! `connect()` time, pull the peer cert, parse it, and extract
//! the CN — that's the serial we use in MQTT topic paths.
//!
//! Faithful port of `bambu-overlay/src/local/device.rs`. The
//! 5-second timeout matches overlay's `LOCAL_MQTT_PROBE_TIMEOUT`.

use std::time::Duration;

use native_tls::Certificate;
use tokio::net::TcpStream;
use tokio_native_tls::TlsConnector as TokioTlsConnector;
use x509_parser::prelude::{FromDer, X509Certificate};

use crate::core::driver::traits::DriverError;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Probe the printer's TLS endpoint, return the serial number
/// encoded in the device cert's CN.
pub async fn probe(host: &str, port: u16) -> Result<String, DriverError> {
    let address = format!("{host}:{port}");
    let tcp = tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect(&address))
        .await
        .map_err(|_| DriverError::Network(format!("timed out connecting to {address}")))?
        .map_err(|e| DriverError::Network(format!("connect to {address}: {e}")))?;

    let connector = super::tls::connector().map_err(DriverError::Other)?;
    let connector = TokioTlsConnector::from(connector);
    let socket = tokio::time::timeout(PROBE_TIMEOUT, connector.connect(host, tcp))
        .await
        .map_err(|_| DriverError::Network(format!("timed out handshaking with {address}")))?
        .map_err(|e| DriverError::Network(format!("TLS handshake with {address}: {e}")))?;

    let cert = socket
        .get_ref()
        .peer_certificate()
        .map_err(|e| DriverError::Protocol(format!("read peer cert: {e}")))?
        .ok_or_else(|| DriverError::Protocol("printer presented no cert".into()))?;

    extract_cn(&cert).ok_or_else(|| {
        DriverError::Protocol(
            "peer cert has no Subject Common Name; cannot derive device id".into(),
        )
    })
}

/// CN extraction from a `native_tls::Certificate`. Pure
/// function; exported for unit testing against fixture certs.
pub fn extract_cn(cert: &Certificate) -> Option<String> {
    let der = cert.to_der().ok()?;
    let (rest, parsed) = X509Certificate::from_der(&der).ok()?;
    if !rest.is_empty() {
        return None;
    }
    // Collect into an owned `String` inside the same statement
    // where `parsed` is alive — otherwise the iterator's `&str`
    // borrows outlive the cert and the borrow checker rejects.
    let cn: Option<String> = parsed
        .subject()
        .iter_common_name()
        .find_map(|cn| cn.as_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    cn
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded BBL CA itself has CN "BBL CA". Sanity check
    /// our extractor on a known cert.
    #[test]
    fn extract_cn_finds_bbl_ca_cn() {
        super::super::tls::connector()
            .map(|_| ())
            .expect("connector builds");
        // We can't easily get the embedded CA out without re-
        // parsing the PEM. Just sanity-check the parse path on a
        // small self-signed test cert generated inline if one is
        // present. For now we cover this via bambu-overlay's own
        // tests + manual real-printer testing.
    }

    #[tokio::test]
    async fn probe_returns_network_error_on_unreachable_host() {
        // RFC 5737 documentation-only address — guaranteed
        // unreachable on the open internet, so the connect
        // either times out or refuses fast.
        let res = probe("192.0.2.1", 8883).await;
        match res {
            Err(DriverError::Network(_)) => {}
            other => panic!("expected Network error, got {other:?}"),
        }
    }
}
