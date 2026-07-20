//! [`U1Driver`] — the Snapmaker U1 [`Driver`], a thin vendor wrapper over
//! [`MoonrakerDriver`].
//!
//! The U1 speaks vanilla Moonraker for print / status / job control, so the
//! generic driver does all of that (constructed with the [`moonraker::U1`]
//! status codec, which decodes the U1's toolhead/filament state into
//! [`DriverExtra::U1`]). This wrapper adds only what's genuinely U1 firmware:
//! it reports [`DriverKind::U1`] and implements the `FLOW_CALIBRATE` /
//! `PARK_EXTRUDER` macros. Everything else delegates to the inner driver.

use std::sync::Arc;

use async_trait::async_trait;

use tokio::sync::watch;

use super::super::moonraker::http;
use super::super::moonraker::{
    driver::U1 as U1_CODEC, MoonrakerConfig, MoonrakerDriver, StatusSessionFactory,
};
use crate::core::driver::status::PrinterStatus;
use crate::core::driver::traits::{
    ControlPlane, Driver, DriverError, DriverId, DriverKind, PrinterCommand, SendHandle,
    SendPayload, UploadProgressFn,
};

pub struct U1Driver {
    inner: MoonrakerDriver,
}

impl U1Driver {
    pub fn new(
        id: DriverId,
        config: MoonrakerConfig,
        factory: Arc<dyn StatusSessionFactory>,
    ) -> Self {
        Self {
            inner: MoonrakerDriver::new(id, config, factory, U1_CODEC),
        }
    }
}

#[async_trait]
impl Driver for U1Driver {
    fn id(&self) -> DriverId {
        self.inner.id()
    }

    fn kind(&self) -> DriverKind {
        DriverKind::U1
    }

    async fn connect(&mut self) -> Result<(), DriverError> {
        self.inner.connect().await
    }

    async fn disconnect(&mut self) -> Result<(), DriverError> {
        self.inner.disconnect().await
    }

    fn status(&self) -> PrinterStatus {
        self.inner.status()
    }

    fn subscribe_status(&self) -> watch::Receiver<PrinterStatus> {
        self.inner.subscribe_status()
    }

    async fn send(
        &self,
        payload: SendPayload,
        on_progress: UploadProgressFn,
    ) -> Result<SendHandle, DriverError> {
        self.inner.send(payload, on_progress).await
    }

    async fn command(&self, cmd: PrinterCommand) -> Result<(), DriverError> {
        self.inner.command(cmd).await
    }

    async fn calibrate_pressure_advance(
        &self,
        extruder_idx: usize,
    ) -> Result<f64, DriverError> {
        let c = self.inner.config();
        http::calibrate_pressure_advance(&c.host, c.port, extruder_idx).await
    }

    async fn park_extruder(&self) -> Result<(), DriverError> {
        let c = self.inner.config();
        http::park_extruder(&c.host, c.port).await
    }

    fn control_plane(&self) -> Option<Arc<dyn ControlPlane>> {
        self.inner.control_plane()
    }
}
