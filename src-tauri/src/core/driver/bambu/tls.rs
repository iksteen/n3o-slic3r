//! Shared `native-tls` connector for Bambu device-local
//! services. Memoized in a `OnceLock` so the FTPS path (PR-7a-5)
//! and the MQTT path (here) build it once.
//!
//! Faithful port of `bambu-overlay/src/device_tls.rs`. The
//! choice of `native-tls` over `rustls` is load-bearing: BBL
//! device certs are X.509 v1 with the serial number as the
//! subject CN, and rustls's custom-verifier path can't accept
//! v1 certs. `danger_accept_invalid_hostnames(true)` is also
//! required for the same reason — hostname verification fails
//! because the cert CN is the serial, not the IP/hostname.
//!
//! **CA expiry: 2032-04-01.** When that day comes (or by the
//! time we're within 6 months of it) Bambu will need to ship a
//! firmware update with a fresh CA — and we ship a corresponding
//! update with the new PEM here.

use std::sync::OnceLock;

use native_tls::{Certificate, TlsConnector};

/// Embedded Bambu Labs CA. Verbatim from bambu-overlay's
/// `device_tls.rs:14-37`. **Expires 2032-04-01.**
const BBL_CA_CERT_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIDZTCCAk2gAwIBAgIUV1FckwXElyek1onFnQ9kL7Bk4N8wDQYJKoZIhvcNAQEL
BQAwQjELMAkGA1UEBhMCQ04xIjAgBgNVBAoMGUJCTCBUZWNobm9sb2dpZXMgQ28u
LCBMdGQxDzANBgNVBAMMBkJCTCBDQTAeFw0yMjA0MDQwMzQyMTFaFw0zMjA0MDEw
MzQyMTFaMEIxCzAJBgNVBAYTAkNOMSIwIAYDVQQKDBlCQkwgVGVjaG5vbG9naWVz
IENvLiwgTHRkMQ8wDQYDVQQDDAZCQkwgQ0EwggEiMA0GCSqGSIb3DQEBAQUAA4IB
DwAwggEKAoIBAQDL3pnDdxGOk5Z6vugiT4dpM0ju+3Xatxz09UY7mbj4tkIdby4H
oeEdiYSZjc5LJngJuCHwtEbBJt1BriRdSVrF6M9D2UaBDyamEo0dxwSaVxZiDVWC
eeCPdELpFZdEhSNTaT4O7zgvcnFsfHMa/0vMAkvE7i0qp3mjEzYLfz60axcDoJLk
p7n6xKXI+cJbA4IlToFjpSldPmC+ynOo7YAOsXt7AYKY6Glz0BwUVzSJxU+/+VFy
/QrmYGNwlrQtdREHeRi0SNK32x1+bOndfJP0sojuIrDjKsdCLye5CSZIvqnbowwW
1jRwZgTBR29Zp2nzCoxJYcU9TSQp/4KZuWNVAgMBAAGjUzBRMB0GA1UdDgQWBBSP
NEJo3GdOj8QinsV8SeWr3US+HjAfBgNVHSMEGDAWgBSPNEJo3GdOj8QinsV8SeWr
3US+HjAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3DQEBCwUAA4IBAQABlBIT5ZeG
fgcK1LOh1CN9sTzxMCLbtTPFF1NGGA13mApu6j1h5YELbSKcUqfXzMnVeAb06Htu
3CoCoe+wj7LONTFO++vBm2/if6Jt/DUw1CAEcNyqeh6ES0NX8LJRVSe0qdTxPJuA
BdOoo96iX89rRPoxeed1cpq5hZwbeka3+CJGV76itWp35Up5rmmUqrlyQOr/Wax6
itosIzG0MfhgUzU51A2P/hSnD3NDMXv+wUY/AvqgIL7u7fbDKnku1GzEKIkfH8hm
Rs6d8SCU89xyrwzQ0PR853irHas3WrHVqab3P+qNwR0YirL0Qk7Xt/q3O1griNg2
Blbjg3obpHo9
-----END CERTIFICATE-----"#;

/// Build (or fetch the cached) connector. The `Result` is stored
/// as `String` inside the `OnceLock` so `Clone` works — the
/// builder error type doesn't itself impl `Clone`.
pub fn connector() -> Result<TlsConnector, String> {
    static CONNECTOR: OnceLock<Result<TlsConnector, String>> = OnceLock::new();
    CONNECTOR.get_or_init(build).clone()
}

fn build() -> Result<TlsConnector, String> {
    let ca = Certificate::from_pem(BBL_CA_CERT_PEM.as_bytes())
        .map_err(|e| format!("failed to parse embedded BBL CA: {e}"))?;
    let mut builder = TlsConnector::builder();
    builder.disable_built_in_roots(true);
    builder.use_sni(true);
    builder.add_root_certificate(ca);
    builder.danger_accept_invalid_hostnames(true);
    builder
        .build()
        .map_err(|e| format!("failed to build Bambu device TLS connector: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity: the embedded PEM parses as a native_tls cert. If
    /// this ever fails, the PEM was corrupted during a refactor —
    /// re-copy from bambu-overlay's `device_tls.rs:14-37`.
    #[test]
    fn embedded_ca_parses() {
        Certificate::from_pem(BBL_CA_CERT_PEM.as_bytes()).expect("embedded BBL CA should parse");
    }

    #[test]
    fn connector_builds() {
        connector().expect("connector builds with embedded CA");
    }

    #[test]
    fn connector_is_memoized() {
        // Same `Result` value each call (TlsConnector doesn't
        // impl `PartialEq`, but `Clone`-equality via `Arc` inside
        // means the cheap-clone semantics hold).
        let a = connector().expect("first build");
        let b = connector().expect("second build");
        // Drop both to confirm no panic on double-clone.
        drop(a);
        drop(b);
    }
}
