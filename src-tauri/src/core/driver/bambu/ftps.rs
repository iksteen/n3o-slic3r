//! FTPS upload to Bambu's local file system.
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

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use suppaftp::tokio::{AsyncNativeTlsConnector, AsyncNativeTlsFtpStream};
use suppaftp::types::FileType;
use suppaftp::Mode;
use tokio::io::{AsyncRead, ReadBuf};

use crate::core::driver::traits::{DriverError, UploadProgressFn};

/// An `AsyncRead` over an in-memory buffer that reports cumulative bytes
/// through a callback as `put_file` pulls them — real upload progress.
/// Reads never park (no actual IO), so this never stalls the task. There's no
/// cancellation logic here: the send is fully async, so a cancelled send just
/// drops the future (a `select!` at the command layer), which aborts the STOR.
struct ProgressReader {
    bytes: Vec<u8>,
    pos: usize,
    total: u64,
    on_progress: UploadProgressFn,
}

impl AsyncRead for ProgressReader {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();
        let remaining = me.bytes.len() - me.pos;
        if remaining == 0 {
            return Poll::Ready(Ok(())); // EOF
        }
        let n = remaining.min(buf.remaining());
        buf.put_slice(&me.bytes[me.pos..me.pos + n]);
        me.pos += n;
        (me.on_progress)(me.pos as u64, me.total);
        Poll::Ready(Ok(()))
    }
}

const FTPS_PORT: u16 = 990;
const FTP_TIMEOUT: Duration = Duration::from_secs(20);

// Files land at the FTPS root — no `/cache/` directory. See module
// docs for the history of why we don't use BBS's `/cache/` convention.

/// Open + log in to the printer's FTPS endpoint. Blocking; the
/// caller is expected to wrap in `tokio::task::spawn_blocking`.
pub async fn connect(
    host: &str,
    access_code: &str,
) -> Result<AsyncNativeTlsFtpStream, DriverError> {
    let address = if host.contains(':') {
        format!("[{host}]:{FTPS_PORT}")
    } else {
        format!("{host}:{FTPS_PORT}")
    };
    let connector =
        AsyncNativeTlsConnector::from(super::tls::async_connector().map_err(DriverError::Other)?);
    // Bound the connect so an unreachable host fails fast rather than hanging on
    // the OS TCP timeout (the async stream has no per-socket read/write timeout
    // knob like the blocking one did).
    let mut client = tokio::time::timeout(
        FTP_TIMEOUT,
        AsyncNativeTlsFtpStream::connect_secure_implicit(address.as_str(), connector, host),
    )
    .await
    .map_err(|_| {
        DriverError::Network(format!(
            "FTPS connect {address}: timed out after {}s",
            FTP_TIMEOUT.as_secs()
        ))
    })?
    .map_err(|e| DriverError::Network(format!("FTPS connect {address}: {e}")))?;
    client.set_passive_nat_workaround(true);
    client.set_mode(Mode::Passive);
    client
        .login("bblp", access_code)
        .await
        .map_err(|e| DriverError::Auth(format!("FTPS login: {e}")))?;
    client
        .transfer_type(FileType::Binary)
        .await
        .map_err(|e| DriverError::Protocol(format!("FTPS set binary: {e}")))?;
    Ok(client)
}

/// Upload `bytes` to the FTPS root as `<remote_name>`. Returns
/// `remote_name` verbatim — the MQTT `project_file` URL is built
/// as `ftp://<remote_name>` (two slashes, no path prefix).
/// Blocking; wrap in `spawn_blocking`.
pub async fn upload(
    client: &mut AsyncNativeTlsFtpStream,
    remote_name: &str,
    bytes: Vec<u8>,
    on_progress: UploadProgressFn,
) -> Result<String, DriverError> {
    let total = bytes.len() as u64;
    let mut reader = ProgressReader {
        bytes,
        pos: 0,
        total,
        on_progress,
    };
    client
        .put_file(remote_name, &mut reader)
        .await
        .map_err(|e| DriverError::Network(format!("FTPS STOR {remote_name}: {e}")))?;
    Ok(remote_name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn progress_reader_reports_monotonic_bytes_up_to_total() {
        let data = vec![7u8; 10_000];
        let seen: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_cb = seen.clone();
        let mut reader = ProgressReader {
            bytes: data.clone(),
            pos: 0,
            total: data.len() as u64,
            on_progress: Arc::new(move |sent, _total| seen_cb.lock().unwrap().push(sent)),
        };
        // Drain in fixed chunks, mimicking how put_file pumps the socket.
        let mut buf = [0u8; 1024];
        loop {
            let n = reader.read(&mut buf).await.unwrap();
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
