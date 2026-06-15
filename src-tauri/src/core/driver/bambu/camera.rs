//! Bambu LAN camera stream — the low-rate "chamber" / frame cam on the
//! A1 mini and its kin.
//!
//! It rides a bespoke binary protocol on TCP `:6000`, behind the *same*
//! BBL-CA device TLS the MQTT (`status`) and FTPS (`ftps`) paths already
//! use ([`super::tls`]). After the handshake the client sends one 80-byte
//! authentication packet (the `bblp` username + the per-device access
//! code) and the printer streams **length-prefixed JPEG frames**: a
//! 16-byte header whose first little-endian `u32` is the frame size,
//! followed by exactly that many JPEG bytes (SOI `FF D8` … EOI `FF D9`).
//!
//! The framing + auth-packet layout are verified against
//! [`iksteen/machin3d-overlay`] (`src/video/{protocol,connection}.rs`),
//! itself packet-capture-derived from the A1 / P1 stream.
//!
//! This module owns only the wire protocol: building the auth packet,
//! validating a frame, and the connect→auth→read loop for a single
//! attempt. Lifecycle (start on panel open, stop on panel hide, retry
//! with backoff) lives in [`crate::core::driver::camera`].
//!
//! [`iksteen/machin3d-overlay`]: https://github.com/iksteen/machin3d-overlay

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_native_tls::TlsConnector as TokioTlsConnector;

use crate::core::driver::traits::DriverError;

/// The printer's video port. Same on every LAN-mode Bambu.
pub const VIDEO_PORT: u16 = 6000;

/// Reject absurd frame sizes before allocating — a corrupt/garbage
/// header otherwise asks us to allocate gigabytes. 16 MiB is far above
/// any real JPEG frame from this camera. Matches the overlay's cap.
const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A live stream emits frames continuously; a gap this long means the
/// link is wedged, so we drop it and let the manager reconnect.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// Build the 80-byte authentication packet the camera server expects
/// before it will stream.
///
/// Layout (verified against the overlay): four little-endian `u32`
/// header words `0x40, 0x3000, 0, 0`, then a 32-byte username field
/// holding the literal `bblp`, then a 32-byte field holding the device
/// access code — both ASCII, zero-padded.
pub fn auth_packet(access_code: &str) -> Result<[u8; 80], DriverError> {
    let mut packet = [0_u8; 80];
    packet[0..4].copy_from_slice(&0x40_u32.to_le_bytes());
    packet[4..8].copy_from_slice(&0x3000_u32.to_le_bytes());
    // packet[8..16] stay zero.
    write_auth_field(&mut packet[16..48], "bblp", "video username")?;
    write_auth_field(&mut packet[48..80], access_code.trim(), "video access code")?;
    Ok(packet)
}

fn write_auth_field(target: &mut [u8], value: &str, label: &str) -> Result<(), DriverError> {
    if !value.is_ascii() {
        return Err(DriverError::Auth(format!("{label} must be ASCII")));
    }
    if value.len() > target.len() {
        return Err(DriverError::Auth(format!(
            "{label} must fit in {} bytes",
            target.len()
        )));
    }
    target[..value.len()].copy_from_slice(value.as_bytes());
    Ok(())
}

/// A frame is a complete JPEG iff it opens with the SOI marker and ends
/// with the EOI marker. The camera occasionally emits a non-JPEG control
/// frame; the caller drops those rather than forwarding garbage to the
/// `<img>`.
pub fn is_jpeg(frame: &[u8]) -> bool {
    frame.starts_with(&[0xff, 0xd8]) && frame.ends_with(&[0xff, 0xd9])
}

/// Run a single connect→authenticate→stream attempt against `host`.
///
/// Each decoded JPEG frame is handed to `on_frame`. The loop runs until
/// `on_frame` returns `false` (the consumer is gone — a clean stop) or an
/// IO/protocol error occurs (returned as `Err` so the caller can back off
/// and retry). Returns `Ok(())` only on a clean consumer-requested stop.
///
/// We reuse [`super::tls`]'s BBL-CA-pinned connector, so the peer is
/// already proven to be a genuine Bambu device; we connect by host (the
/// device cert's CN is the serial, not the IP, hence the connector's
/// `danger_accept_invalid_hostnames`). The serial is read from the peer
/// cert and logged for diagnostics, not enforced here — the manager keys
/// the worker by the same instance whose connection settings supplied
/// `host`.
pub async fn stream_once<F>(
    host: &str,
    access_code: &str,
    mut on_frame: F,
) -> Result<(), DriverError>
where
    F: FnMut(Vec<u8>) -> bool,
{
    let address = format!("{host}:{VIDEO_PORT}");
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&address))
        .await
        .map_err(|_| DriverError::Network(format!("timed out connecting to {address}")))?
        .map_err(|e| DriverError::Network(format!("connect to {address}: {e}")))?;

    let connector = super::tls::connector().map_err(DriverError::Other)?;
    let connector = TokioTlsConnector::from(connector);
    let mut socket = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(host, tcp))
        .await
        .map_err(|_| DriverError::Network(format!("timed out handshaking with {address}")))?
        .map_err(|e| DriverError::Network(format!("TLS handshake with {address}: {e}")))?;

    if let Ok(Some(cert)) = socket.get_ref().peer_certificate() {
        if let Some(serial) = super::device_id::extract_cn(&cert) {
            tracing::debug!(serial = %serial, %address, "camera stream connected");
        }
    }

    socket
        .write_all(&auth_packet(access_code)?)
        .await
        .map_err(|e| DriverError::Network(format!("send camera auth packet: {e}")))?;
    socket
        .flush()
        .await
        .map_err(|e| DriverError::Network(format!("flush camera auth packet: {e}")))?;

    let mut header = [0_u8; 16];
    loop {
        read_exact_timed(&mut socket, &mut header, "camera frame header").await?;
        let frame_size =
            u32::from_le_bytes(header[0..4].try_into().expect("4-byte slice")) as usize;
        if !(1..=MAX_FRAME_SIZE).contains(&frame_size) {
            return Err(DriverError::Protocol(format!(
                "invalid camera frame size {frame_size}"
            )));
        }

        let mut frame = vec![0_u8; frame_size];
        read_exact_timed(&mut socket, &mut frame, "camera frame").await?;

        if is_jpeg(&frame) {
            if !on_frame(frame) {
                // Consumer (the webview channel) is gone — clean stop.
                return Ok(());
            }
        } else {
            tracing::warn!("discarding camera frame without JPEG markers");
        }
    }
}

async fn read_exact_timed<S>(socket: &mut S, buf: &mut [u8], label: &str) -> Result<(), DriverError>
where
    S: AsyncReadExt + Unpin,
{
    tokio::time::timeout(READ_TIMEOUT, socket.read_exact(buf))
        .await
        .map_err(|_| DriverError::Network(format!("timed out reading {label}")))?
        .map_err(|e| DriverError::Network(format!("read {label}: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{auth_packet, is_jpeg};

    #[test]
    fn auth_packet_matches_a1_protocol_layout() {
        let packet = auth_packet("12345678").expect("access code fits");

        assert_eq!(&packet[0..4], &0x40_u32.to_le_bytes());
        assert_eq!(&packet[4..8], &0x3000_u32.to_le_bytes());
        assert_eq!(&packet[8..16], &[0_u8; 8]);
        assert_eq!(&packet[16..20], b"bblp");
        assert!(packet[20..48].iter().all(|b| *b == 0));
        assert_eq!(&packet[48..56], b"12345678");
        assert!(packet[56..80].iter().all(|b| *b == 0));
    }

    #[test]
    fn auth_packet_trims_access_code() {
        let packet = auth_packet("  12345678  ").expect("trimmed code fits");
        assert_eq!(&packet[48..56], b"12345678");
        assert!(packet[56..80].iter().all(|b| *b == 0));
    }

    #[test]
    fn auth_packet_rejects_oversized_access_code() {
        let err = auth_packet(&"x".repeat(33)).unwrap_err();
        assert!(err.to_string().contains("video access code"));
    }

    #[test]
    fn auth_packet_rejects_non_ascii_access_code() {
        let err = auth_packet("café-code").unwrap_err();
        assert!(err.to_string().contains("ASCII"));
    }

    #[test]
    fn jpeg_check_requires_soi_and_eoi() {
        assert!(is_jpeg(&[0xff, 0xd8, 0x00, 0xff, 0xd9]));
        assert!(!is_jpeg(&[0xff, 0xd8, 0x00])); // no EOI
        assert!(!is_jpeg(&[0x00, 0xff, 0xd9])); // no SOI
        assert!(!is_jpeg(&[])); // empty
    }
}
