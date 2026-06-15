//! Snapmaker U1 LAN driver (PR-7b-*).
//!
//! Built against the user's [`iksteen/bambu-overlay`] reference
//! implementation's snapmaker module — every probe / WebSocket /
//! status-decode pattern in this module mirrors the overlay's
//! read-only side. Send + commands (PR-7b-4) talk to the same
//! Moonraker HTTP endpoints any Klipper-based slicer driver
//! consumes (`/server/files/upload`, `/printer/print/start`,
//! `/printer/print/{pause,resume,cancel}`).
//!
//! **Architecture note** (supersedes the stale "not Moonraker"
//! statement in `core/printer/snapmaker/mod.rs` and PRD AD-7):
//! the U1 actually exposes a vanilla Moonraker endpoint over
//! plain HTTP+WS on port 80. The Snapmaker-specific pair/mTLS
//! dance is webcam-only — print + status ride the plain Moonraker
//! endpoint — and is implemented here (`pairing`, `mtls`,
//! `snap_token`, `camera`) to drive the live camera.
//!
//! [`iksteen/bambu-overlay`]: https://github.com/iksteen/bambu-overlay

pub mod camera;
pub mod commands;
pub mod connection;
pub mod http;
pub mod moonraker;
pub mod mtls;
pub mod pairing;
pub mod probe;
pub mod snap_token;
pub mod status;

pub use connection::{U1Config, U1Driver};
