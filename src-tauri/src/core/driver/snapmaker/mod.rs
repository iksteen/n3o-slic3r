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
//! dance the overlay also implements is webcam-only and out of
//! scope here. PR-7b-6 updates the doc.
//!
//! [`iksteen/bambu-overlay`]: https://github.com/iksteen/bambu-overlay

pub mod probe;
