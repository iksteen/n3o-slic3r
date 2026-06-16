# Vendored suppaftp 8.0.3 — Windows-portability patch

This is an unmodified copy of [`suppaftp`](https://crates.io/crates/suppaftp)
8.0.3 with a **single** change, wired in via `[patch.crates.io]` in the
workspace root `Cargo.toml`.

## Why

n3o's Bambu driver uploads over FTPS on suppaftp's **tokio**
(`tokio-async-native-tls`) async path, so a send can be cancelled by dropping
the future. But upstream's `AsyncNativeTlsStream::tcp_stream()` uses
`std::os::fd::{AsFd, ...}` unconditionally, and `std::os::fd` doesn't exist on
Windows — so the crate fails to compile for `x86_64-pc-windows-msvc` (a hard
`E0432`/`E0599`), breaking the Windows cross-build. The bug is unchanged in
8.0.4.

## The change

`src/async_ftp/tokio_ftp/tls/native_tls.rs` — `tcp_stream()` now duplicates the
underlying socket through the platform's owned-handle API: `AsFd` on Unix (as
before) and `AsSocket` (`std::os::windows::io`) on Windows. Both yield an owned
handle that `std::net::TcpStream` accepts, so the rest of the method is
unchanged. This mirrors what the sync path already does cross-platform.

Diff against upstream is limited to that one method. To refresh on a suppaftp
bump: re-copy the crate and re-apply this `#[cfg(unix)] / #[cfg(windows)]`
split (or drop the vendoring entirely once upstream guards it).
