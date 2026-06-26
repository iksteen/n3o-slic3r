//! Bambu A1 mini LAN driver.
//!
//! Built against the user's [`iksteen/bambu-overlay`] reference
//! implementation — every connection / TLS / pushall pattern in
//! this module is a faithful port of overlay's read-only side.
//! Send + commands reference BambuStudio's own source; the
//! overlay is status-only.
//!
//! [`iksteen/bambu-overlay`]: https://github.com/iksteen/bambu-overlay

pub mod camera;
pub mod connection;
pub mod device_id;
pub mod ftps;
pub mod status;
pub mod tls;

pub use connection::BambuDriver;
pub use status::{parse_message, BambuMessage, BambuReport};
