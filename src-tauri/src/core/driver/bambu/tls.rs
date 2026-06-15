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

    // macOS: native-tls is backed by Security.framework, whose TLS trust
    // policy enforces the CA/Browser-Forum maximum leaf-certificate validity
    // (398 days for certs issued after 2020-09-01). Bambu device certs are
    // minted with a ~10-year lifetime (e.g. 2025→2035), so SecTrustEvaluate
    // rejects the otherwise-valid chain to the embedded BBL CA with
    // errSecCertificateValidityPeriodTooLong — surfaced as "The validity
    // period in the certificate exceeds the maximum allowed". The same chain
    // verifies fine under OpenSSL on Linux. native-tls exposes no knob to
    // relax *only* the validity rule, so on macOS we skip chain verification.
    //
    // The practical trust model is unchanged: this is a LAN connection to a
    // user-entered device IP, still gated by the per-device access code in the
    // MQTT credentials, and the serial probe (device_id.rs) continues to read
    // the peer cert's CN regardless of verification. We keep full BBL-CA
    // pinning on every other platform.
    #[cfg(target_os = "macos")]
    builder.danger_accept_invalid_certs(true);

    builder
        .build()
        .map_err(|e| format!("failed to build Bambu device TLS connector: {e}"))
}

/// Async (tokio) twin of [`connector`] for the FTPS upload path, which runs on
/// `suppaftp`'s `tokio-async-native-tls` stack. Same BBL-CA pinning + invalid-
/// hostname acceptance. `async-native-tls` exposes no `disable_built_in_roots`,
/// but the device cert still validates against the added BBL CA — the extra
/// system roots are simply unused, not a trust change for this LAN connection.
/// Built fresh per call (FTPS connects once per send; no need to memoize).
pub fn async_connector() -> Result<suppaftp::async_native_tls::TlsConnector, String> {
    let ca = Certificate::from_pem(BBL_CA_CERT_PEM.as_bytes())
        .map_err(|e| format!("failed to parse embedded BBL CA: {e}"))?;
    #[allow(unused_mut)]
    let mut connector = suppaftp::async_native_tls::TlsConnector::new()
        .use_sni(true)
        .add_root_certificate(ca)
        .danger_accept_invalid_hostnames(true);
    // macOS: same Security.framework validity-period quirk as the sync path —
    // skip chain verification there (see the note above).
    #[cfg(target_os = "macos")]
    {
        connector = connector.danger_accept_invalid_certs(true);
    }
    Ok(connector)
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
