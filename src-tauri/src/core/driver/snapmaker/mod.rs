//! Snapmaker U1 vendor layer on top of the generic Moonraker driver.
//!
//! The U1 exposes a vanilla Moonraker endpoint over plain HTTP+WS on
//! port 80 — print, status, and job control all ride
//! [`super::moonraker::MoonrakerDriver`] registered with
//! `DriverKind::U1`. What lives here is only the Snapmaker-specific
//! webcam stack: the camera daemon sits behind a bespoke per-device
//! pair/mTLS dance ([`pairing`], [`mtls`], [`snap_token`]) and a
//! monitor-mode wake ([`camera`]); the JPEG poll itself is the
//! generic Moonraker one (`super::moonraker::webcam`).
//!
//! Built against the user's [`iksteen/bambu-overlay`] reference
//! implementation's snapmaker module.
//!
//! [`iksteen/bambu-overlay`]: https://github.com/iksteen/bambu-overlay

pub mod camera;
pub mod commands;
pub mod mtls;
pub mod pairing;
pub mod snap_token;
