//! FTPS upload to Bambu's local file system (PR-7a-5).
//!
//! Implicit TLS on port 990, login `bblp` + LAN access code,
//! passive mode + NAT workaround, binary transfer type.
//! Faithful port of bambu-overlay's `connect_local_ftps`
//! (`src/thumbnail/local.rs` — overlay uses it for download
//! only; upload via `STOR` is the same connection shape).
//!
//! Uploaded files land under `/cache/<name>` on the printer's
//! SD card — same convention as Bambu Studio. The MQTT
//! `project_file` command (in `connection.rs`) references the
//! uploaded file via `file:///mnt/sdcard/cache/<name>`.

use std::io::Cursor;
use std::time::Duration;

use suppaftp::types::FileType;
use suppaftp::{Mode, NativeTlsConnector, NativeTlsFtpStream};

use crate::core::driver::traits::DriverError;

const FTPS_PORT: u16 = 990;
const FTP_TIMEOUT: Duration = Duration::from_secs(20);

/// Where uploaded `.gcode.3mf` files live on the printer.
/// Matches Bambu Studio + bambu-overlay's lookup order.
pub const REMOTE_CACHE_DIR: &str = "/cache";

/// Open + log in to the printer's FTPS endpoint. Blocking; the
/// caller is expected to wrap in `tokio::task::spawn_blocking`.
pub fn connect(host: &str, access_code: &str) -> Result<NativeTlsFtpStream, DriverError> {
    let address = if host.contains(':') {
        format!("[{host}]:{FTPS_PORT}")
    } else {
        format!("{host}:{FTPS_PORT}")
    };
    let connector =
        NativeTlsConnector::from(super::tls::connector().map_err(DriverError::Other)?);
    let mut client = NativeTlsFtpStream::connect_secure_implicit(
        address.as_str(),
        connector,
        host,
    )
    .map_err(|e| DriverError::Network(format!("FTPS connect {address}: {e}")))?;
    client
        .get_ref()
        .set_read_timeout(Some(FTP_TIMEOUT))
        .map_err(|e| DriverError::Network(format!("FTPS read timeout: {e}")))?;
    client
        .get_ref()
        .set_write_timeout(Some(FTP_TIMEOUT))
        .map_err(|e| DriverError::Network(format!("FTPS write timeout: {e}")))?;
    client.set_passive_nat_workaround(true);
    client.set_mode(Mode::Passive);
    client
        .login("bblp", access_code)
        .map_err(|e| DriverError::Auth(format!("FTPS login: {e}")))?;
    client
        .transfer_type(FileType::Binary)
        .map_err(|e| DriverError::Protocol(format!("FTPS set binary: {e}")))?;
    Ok(client)
}

/// Upload `bytes` to `<REMOTE_CACHE_DIR>/<remote_name>`. The
/// printer's MQTT `project_file` command then references this
/// path via `file:///mnt/sdcard/cache/<remote_name>`. Blocking;
/// wrap in `spawn_blocking`.
pub fn upload(
    client: &mut NativeTlsFtpStream,
    remote_name: &str,
    bytes: &[u8],
) -> Result<String, DriverError> {
    // CWD into the cache directory. The printer auto-creates it
    // on first upload from Bambu Studio; we tolerate the case
    // where it already exists (CWD returns 250) and the case
    // where we need to MKD it first.
    if client.cwd(REMOTE_CACHE_DIR).is_err() {
        client
            .mkdir(REMOTE_CACHE_DIR)
            .map_err(|e| DriverError::Protocol(format!("FTPS MKDIR {REMOTE_CACHE_DIR}: {e}")))?;
        client
            .cwd(REMOTE_CACHE_DIR)
            .map_err(|e| DriverError::Protocol(format!("FTPS CWD {REMOTE_CACHE_DIR}: {e}")))?;
    }
    let mut cursor = Cursor::new(bytes);
    client
        .put_file(remote_name, &mut cursor)
        .map_err(|e| DriverError::Network(format!("FTPS STOR {remote_name}: {e}")))?;
    Ok(format!("{REMOTE_CACHE_DIR}/{remote_name}"))
}

/// MD5 hex digest of the body. Bambu's project_file command
/// has an `md5` field for integrity verification. We share the
/// implementation with PR-3-10's sliced.rs (which already
/// embeds a self-contained MD5 — md5 is broken cryptographically
/// but Bambu firmware insists on it for the plate checksum).
pub fn md5_hex(bytes: &[u8]) -> String {
    crate::core::threemf::md5_hex(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_cache_dir_matches_bambu_studio_convention() {
        // bambu-overlay's `local_file_candidates` tries
        // `/cache/<name>` first, then `/model/<name>`. We upload
        // to the `cache/` candidate so subsequent downloads
        // (overlay-style) find our file on the first try.
        assert_eq!(REMOTE_CACHE_DIR, "/cache");
    }

    #[test]
    fn md5_hex_matches_rfc1321_test_vector() {
        // Sanity: shared MD5 impl works.
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
    }
}
