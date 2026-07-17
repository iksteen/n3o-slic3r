//! Generic Moonraker (Klipper) LAN driver.
//!
//! Speaks the vanilla Moonraker API any Klipper-based printer
//! exposes: JSON-RPC status over WebSocket ([`session`]), HTTP
//! upload + job control ([`http`]), the `/machine/system_info`
//! connectivity probe ([`probe`]), and webcam discovery via
//! `/server/webcams/list` ([`webcam`]).
//!
//! [`MoonrakerDriver`] is the [`crate::core::driver::traits::Driver`]
//! impl; it's constructed with the [`DriverKind`] it reports so
//! vendor drivers built on Moonraker reuse it wholesale — the
//! Snapmaker U1 registers one with `DriverKind::U1` and tacks on
//! only its bespoke webcam stack (`super::snapmaker`: pairing,
//! mTLS, monitor-mode wake).
//!
//! [`DriverKind`]: crate::core::driver::traits::DriverKind

pub mod driver;
pub mod http;
pub mod probe;
pub mod session;
pub mod status;
pub mod transport;
pub mod webcam;

pub use driver::{MoonrakerConfig, MoonrakerDriver};
pub use session::WsSessionFactory;
pub use transport::{StatusSession, StatusSessionFactory};
