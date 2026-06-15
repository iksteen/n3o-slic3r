//! Build a rumqttc [`Transport`] from a paired U1's mTLS material.
//!
//! The Bambu analogue is `bambu::tls`, but there the trust anchor is a
//! hard-coded vendor CA; here it's the printer-issued CA from a paired
//! [`SnapToken`], plus our client identity (cert + key). Hostname
//! verification is disabled because the printer's server cert is keyed to
//! its serial, not the LAN host/IP we dial.
//!
//! Faithful port of `iksteen/machin3d-overlay`'s `moonraker/u1/mtls.rs`.

use native_tls::{Certificate, Identity, TlsConnector};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::{EncodePrivateKey, LineEnding};
use rumqttc::Transport;

use super::snap_token::SnapToken;
use crate::core::driver::traits::DriverError;

/// Build an mTLS transport for the paired printer: trust its CA, present
/// our client identity.
pub fn transport_for(token: &SnapToken) -> Result<Transport, DriverError> {
    let ca = Certificate::from_pem(token.ca.as_bytes())
        .map_err(|e| DriverError::Other(format!("parse U1 mTLS CA: {e}")))?;
    let key_pkcs8 = normalize_key_to_pkcs8_pem(&token.key)?;
    let identity = Identity::from_pkcs8(token.cert.as_bytes(), key_pkcs8.as_bytes())
        .map_err(|e| DriverError::Other(format!("build U1 mTLS client identity: {e}")))?;

    let connector = TlsConnector::builder()
        .disable_built_in_roots(true)
        .add_root_certificate(ca)
        .identity(identity)
        .use_sni(true)
        // The server cert is keyed to the SN, not the LAN host we dial.
        .danger_accept_invalid_hostnames(true)
        .build()
        .map_err(|e| DriverError::Other(format!("build U1 mTLS connector: {e}")))?;
    Ok(Transport::tls_with_config(connector.into()))
}

/// native-tls's `Identity::from_pkcs8` strictly requires the PKCS#8 PEM
/// label (`-----BEGIN PRIVATE KEY-----`). The U1 ships PKCS#1 RSA keys
/// (`-----BEGIN RSA PRIVATE KEY-----`); re-wrap them in two trait calls.
fn normalize_key_to_pkcs8_pem(pem: &str) -> Result<String, DriverError> {
    let trimmed = pem.trim();
    if trimmed.contains("-----BEGIN PRIVATE KEY-----") {
        return Ok(trimmed.to_owned());
    }
    if trimmed.contains("-----BEGIN RSA PRIVATE KEY-----") {
        let key = rsa::RsaPrivateKey::from_pkcs1_pem(trimmed)
            .map_err(|e| DriverError::Other(format!("parse PKCS#1 RSA key: {e}")))?;
        return key
            .to_pkcs8_pem(LineEnding::LF)
            .map(|s| s.to_string())
            .map_err(|e| DriverError::Other(format!("encode key as PKCS#8: {e}")));
    }
    Err(DriverError::Other(
        "unsupported private key format; expected a PEM `PRIVATE KEY` or `RSA PRIVATE KEY`"
            .to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkcs8_pem_passes_through_unchanged() {
        let input = "-----BEGIN PRIVATE KEY-----\nABCD\n-----END PRIVATE KEY-----\n";
        assert_eq!(normalize_key_to_pkcs8_pem(input).unwrap(), input.trim());
    }

    #[test]
    fn rejects_unknown_pem_label() {
        let input = "-----BEGIN EC PRIVATE KEY-----\nABCD\n-----END EC PRIVATE KEY-----\n";
        let err = normalize_key_to_pkcs8_pem(input).unwrap_err();
        assert!(err.to_string().contains("unsupported private key format"));
    }

    #[test]
    fn pkcs1_rsa_pem_is_rewrapped_into_pkcs8() {
        // 1024-bit RSA test key (`openssl genrsa -traditional 1024`).
        let pkcs1 = r#"-----BEGIN RSA PRIVATE KEY-----
MIICXQIBAAKBgQCzmFoHvoOyU0OBjRu57QDEN1J9Ln+PF+uKAD54VaiTEozLcoFn
k2s6+zNob+KrN/ecfQiyIQzLzI9dhEe62tsYq9wmmxsFkLxOTM+R7h9nhq10QOmn
uzyKnJ70aYoesXoj7bH14JjSWtXwVoAVZvd1FLkBGHzeP+5tK4w3d30ROwIDAQAB
AoGBAKZFf9y5mm4HvnD7tla9QL9oxJsW6IwPRkc+kJeSHn8DZoyY14uQJW+2z9J5
+64vI7Si4eEgzhsEqRqYdFxfcQVpjHP9Cb9Twm8ZQ7jiNwVZUNKIJOT0wsACAg+0
Uh4qfzSTAjPcUV4k0eOcuzimA/q0+9cpwumDiFmwu2u+E2Z5AkEA5GLjNuObc0KW
QvIAn99QmWRwQhBnf/+uJOkPJB27879SaldFDzBotXrjM6K0yr5UXhLgCn7sxLWk
6B/iVnz4PQJBAMlPQkNLj0WBCAgZMVYDmt01l08c58XpVJfvvfan52ciISQj99zo
j33JsZ1pghJcwpZlIsyCeLAeVdwBgSzeTtcCQQCFs/qu5JrZ5E6RjJme/p5x3qH1
myLshWOOyj4J97pT3VrDVKniVYXHUNT4IrXSx5AertAodNvp4SlUl23rEihFAkAM
tre9nkkHH7YNJOIrx4CBVgAfW/j7U9gm3FpH+KSxq8MiEC94QSvGyvUvttkjJb6Y
VvzSo67RmKjdgy7QUZ3zAkBlaJ7H4zT23ow2bQfzIi0eTPZXhwHLi1mia2d+jsJH
metxN2tLlxN1XW/RyvAjP0YdRUyCPPt/8HAoJQpS9KCf
-----END RSA PRIVATE KEY-----
"#;
        let pkcs8 = normalize_key_to_pkcs8_pem(pkcs1).expect("rewrap PKCS#1 → PKCS#8");
        assert!(pkcs8.contains("-----BEGIN PRIVATE KEY-----"));
        assert!(pkcs8.contains("-----END PRIVATE KEY-----"));
    }
}
