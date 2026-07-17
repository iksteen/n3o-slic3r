//! Vendor-agnostic status transport for the Moonraker driver.
//!
//! The driver worker ([`super::driver`]) streams status without knowing
//! how the bytes arrive: it drives a [`StatusSession`] and reconnects by
//! asking a [`StatusSessionFactory`]. The generic WebSocket transport
//! lives next to it in [`super::session`]; the Snapmaker U1's mTLS MQTT
//! transport lives in the vendor layer (`crate::core::driver::snapmaker`)
//! and is injected at construction, so `moonraker` never depends on
//! `snapmaker`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::core::driver::traits::ControlPlane;
use crate::core::driver::DriverError;

/// One connected status stream. Both the WebSocket and the U1 MQTT
/// transports produce the same merged Moonraker status maps, so the
/// worker consumes them identically.
#[async_trait]
pub trait StatusSession: Send {
    /// The current merged status snapshot.
    fn status(&self) -> Map<String, Value>;

    /// Block until the next status update. `Ok(None)` on a clean close so
    /// the worker reconnects; `Err` on a transport failure.
    async fn next_status(&mut self) -> Result<Option<Map<String, Value>>, DriverError>;

    /// A cloneable send handle over this session's connection, for
    /// fire-and-forget vendor requests (the U1 camera wake). Sends on it
    /// fail once the session dies; callers re-fetch a fresh handle from
    /// the driver rather than retrying a dead one.
    fn control(&self) -> Arc<dyn ControlPlane>;
}

/// Opens a fresh [`StatusSession`] on demand. Held by the driver so it can
/// reconnect after a drop without re-deriving connection details.
#[async_trait]
pub trait StatusSessionFactory: Send + Sync {
    async fn connect(&self) -> Result<Box<dyn StatusSession>, DriverError>;
}
