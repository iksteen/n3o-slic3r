//! FTPS upload to Bambu's local file system (PR-7a-5).
//!
//! Implicit TLS on port 990, login `bblp` + LAN access code,
//! passive mode + NAT workaround, binary transfer type.
//! Faithful port of bambu-overlay's `connect_local_ftps`
//! (`src/thumbnail/local.rs` — overlay uses it for download
//! only; upload via `STOR` is the same connection shape).
//!
//! Uploaded files land at the FTPS root as `<name>`. The MQTT
//! `project_file` command references them via `ftp://<name>` (two
//! slashes, just the filename — bambu-connect's convention).
//!
//! We tried `/cache/<name>` + `ftp:///cache/<name>` first (matches
//! Bambu Studio's convention) — printer accepted the MQTT command
//! but the print engine couldn't resolve the URL and errored
//! "MicroSD R/W exception". Switched to `ftp://cache/<name>` (two
//! slashes, `cache` as path segment) — that worked for the script
//! once, then the cancel mode shifted to firmware-side. Easier to
//! just drop `/cache/` entirely and upload to root.

use std::io::Cursor;
use std::time::Duration;

use suppaftp::types::FileType;
use suppaftp::{Mode, NativeTlsConnector, NativeTlsFtpStream};

use crate::core::driver::traits::DriverError;

const FTPS_PORT: u16 = 990;
const FTP_TIMEOUT: Duration = Duration::from_secs(20);

// Files land at the FTPS root — no `/cache/` directory. See module
// docs for the history of why we don't use BBS's `/cache/` convention.

/// Open + log in to the printer's FTPS endpoint. Blocking; the
/// caller is expected to wrap in `tokio::task::spawn_blocking`.
pub fn connect(host: &str, access_code: &str) -> Result<NativeTlsFtpStream, DriverError> {
    let address = if host.contains(':') {
        format!("[{host}]:{FTPS_PORT}")
    } else {
        format!("{host}:{FTPS_PORT}")
    };
    let connector = NativeTlsConnector::from(super::tls::connector().map_err(DriverError::Other)?);
    let mut client = NativeTlsFtpStream::connect_secure_implicit(address.as_str(), connector, host)
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

/// Upload `bytes` to the FTPS root as `<remote_name>`. Returns
/// `remote_name` verbatim — the MQTT `project_file` URL is built
/// as `ftp://<remote_name>` (two slashes, no path prefix).
/// Blocking; wrap in `spawn_blocking`.
pub fn upload(
    client: &mut NativeTlsFtpStream,
    remote_name: &str,
    bytes: &[u8],
) -> Result<String, DriverError> {
    let mut cursor = Cursor::new(bytes);
    client
        .put_file(remote_name, &mut cursor)
        .map_err(|e| DriverError::Network(format!("FTPS STOR {remote_name}: {e}")))?;
    Ok(remote_name.to_owned())
}
