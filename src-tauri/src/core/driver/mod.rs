//! Printer-driver abstraction (PR-7a-1).
//!
//! The two MVP drivers (Bambu A1 mini over LAN MQTT, Snapmaker
//! U1 over Moonraker WebSocket) share zero protocol but the
//! same lifecycle, the same status surface, and the same
//! command surface. The [`Driver`] trait pins that contract.
//!
//! Design rationale (also called out in `docs/tickets/phase-7.md`):
//!
//! - **One Tauri command set, dispatched by `DriverId`.** Frontend
//!   never calls `bambu_send_print` / `u1_send_print` directly —
//!   it calls [`commands::driver_send`] with the appropriate
//!   id and a [`SendPayload`] variant.
//! - **`PrinterStatus` is union-shaped.** Every driver fills the
//!   common fields (state, current layer, temps); driver-specific
//!   extras live in [`DriverExtra`] with one variant per driver.
//!   Frontend reads common fields generically + branches on
//!   `extra` for AMS / toolhead detail.
//! - **Pre-emptively shaped for the Phase 8 plugin host.** When
//!   drivers become plugins, the trait shape stays the same; only
//!   the implementation surface (in-process vs IPC) changes.

pub mod bambu;
pub mod commands;
pub mod registry;
pub mod status;
pub mod traits;

pub use registry::DriverRegistry;
pub use status::{
    ConnectionState, DriverExtra, JobProgress, JobState, PrinterStatus,
    TempReading, Temps,
};
pub use traits::{
    Driver, DriverConfig, DriverError, DriverId, DriverKind, PrinterCommand,
    SendHandle, SendPayload,
};
