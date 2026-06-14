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

use std::io::{Cursor, Read};
use std::time::Duration;

use suppaftp::types::FileType;
use suppaftp::{Mode, NativeTlsConnector, NativeTlsFtpStream};

use crate::core::driver::traits::{DriverError, UploadProgressFn};

/// A `Read` wrapper that reports cumulative bytes read through a callback —
/// `put_file` pumps it to the data socket, so this reflects real upload
/// progress. `(bytes_sent, total)` after each non-empty read.
struct ProgressReader<R> {
    inner: R,
    sent: u64,
    total: u64,
    on_progress: UploadProgressFn,
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.sent += n as u64;
            (self.on_progress)(self.sent, self.total);
        }
        Ok(n)
    }
}

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
    on_progress: UploadProgressFn,
) -> Result<String, DriverError> {
    let total = bytes.len() as u64;
    let mut reader = ProgressReader {
        inner: Cursor::new(bytes),
        sent: 0,
        total,
        on_progress,
    };
    client
        .put_file(remote_name, &mut reader)
        .map_err(|e| DriverError::Network(format!("FTPS STOR {remote_name}: {e}")))?;
    Ok(remote_name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::{Arc, Mutex};

    #[test]
    fn progress_reader_reports_monotonic_bytes_up_to_total() {
        let data = vec![7u8; 10_000];
        let seen: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_cb = seen.clone();
        let mut reader = ProgressReader {
            inner: Cursor::new(&data[..]),
            sent: 0,
            total: data.len() as u64,
            on_progress: Arc::new(move |sent, _total| seen_cb.lock().unwrap().push(sent)),
        };
        // Drain in fixed chunks, mimicking how put_file pumps the socket.
        let mut buf = [0u8; 1024];
        loop {
            let n = reader.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
        }
        let seen = seen.lock().unwrap();
        assert!(!seen.is_empty(), "callback fired");
        assert!(seen.windows(2).all(|w| w[0] < w[1]), "monotonic increase");
        assert_eq!(*seen.last().unwrap(), data.len() as u64, "ends at total");
    }
}
