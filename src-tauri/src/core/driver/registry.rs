//! [`DriverRegistry`] — Tauri-managed catalog of live drivers.
//!
//! Owned by the Tauri runtime as `State<Arc<DriverRegistry>>`.
//! Concurrency model: the registry itself uses an internal
//! `Mutex` over the id-allocator + the map. Each driver is
//! stored as `Arc<RwLock<Box<dyn Driver>>>` so command handlers
//! can `get()` an owned handle and lock it for one command.
//!
//! The `&self` trait methods (`status`, `send`, `command`,
//! `set_ams_filament`) take a **read** lock, so they run concurrently —
//! a long upload no longer blocks a status poll or a pause/stop on the
//! same driver. Only the `&mut self` lifecycle methods (`connect` /
//! `disconnect`) take the **write** lock, serializing against everything
//! (rare). Different drivers never block each other (separate locks).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::RwLock;

use super::traits::{Driver, DriverId};

/// Tauri-managed catalog. See module-level docs.
pub struct DriverRegistry {
    inner: Mutex<Inner>,
}

struct Inner {
    next_id: u64,
    drivers: HashMap<DriverId, Entry>,
}

struct Entry {
    driver: Arc<RwLock<Box<dyn Driver>>>,
}

impl Default for DriverRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DriverRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                next_id: 1,
                drivers: HashMap::new(),
            }),
        }
    }

    /// Allocate a fresh id + insert the driver. The caller is
    /// responsible for the driver having been initialized
    /// (constructor ran) but NOT for having called `connect()`
    /// yet — that's a separate Tauri command.
    pub fn register(&self, driver: Box<dyn Driver>) -> DriverId {
        self.register_with(|_id| driver)
    }

    /// Allocate the next id, hand it to `builder`, and insert the
    /// driver under that id atomically. Use this when the driver's
    /// own `id()` should match the registry's id (most cases —
    /// drivers carry the id into log spans + outgoing protocol
    /// frames).
    pub fn register_with<F>(&self, builder: F) -> DriverId
    where
        F: FnOnce(DriverId) -> Box<dyn Driver>,
    {
        let mut inner = self.inner.lock().expect("registry mutex");
        let id = DriverId(inner.next_id);
        inner.next_id += 1;
        let driver = builder(id);
        inner.drivers.insert(
            id,
            Entry {
                driver: Arc::new(RwLock::new(driver)),
            },
        );
        id
    }

    /// Remove a driver from the registry. The caller should
    /// have already called `disconnect()` — `remove` doesn't
    /// drive the disconnect itself because doing so under the
    /// registry mutex would block other commands.
    pub fn remove(&self, id: DriverId) -> bool {
        let mut inner = self.inner.lock().expect("registry mutex");
        inner.drivers.remove(&id).is_some()
    }

    /// Grab an `Arc` handle to the driver. The caller `read()`s the inner
    /// `RwLock` for the `&self` methods, `write()`s it for connect/disconnect.
    pub fn get(&self, id: DriverId) -> Option<Arc<RwLock<Box<dyn Driver>>>> {
        let inner = self.inner.lock().expect("registry mutex");
        inner.drivers.get(&id).map(|e| e.driver.clone())
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::driver::status::{BambuExtra, ConnectionState, DriverExtra, PrinterStatus};
    use crate::core::driver::traits::{
        DriverError, DriverKind, PrinterCommand, SendHandle, SendPayload, UploadProgressFn,
    };
    use async_trait::async_trait;
    use tokio::sync::watch;

    /// Minimal stub driver for registry / trait tests. Doesn't
    /// touch any network; just records what was called for
    /// assertion.
    struct StubDriver {
        id: DriverId,
        kind: DriverKind,
        sender: watch::Sender<PrinterStatus>,
        receiver: watch::Receiver<PrinterStatus>,
    }

    impl StubDriver {
        fn new(kind: DriverKind) -> Self {
            let initial = PrinterStatus::disconnected_for(match kind {
                DriverKind::Bambu => DriverExtra::Bambu(BambuExtra::default()),
                DriverKind::U1 => DriverExtra::U1(Default::default()),
            });
            let (sender, receiver) = watch::channel(initial);
            Self {
                id: DriverId(0),
                kind,
                sender,
                receiver,
            }
        }
    }

    #[async_trait]
    impl Driver for StubDriver {
        fn id(&self) -> DriverId {
            self.id
        }
        fn kind(&self) -> DriverKind {
            self.kind
        }
        async fn connect(&mut self) -> Result<(), DriverError> {
            let mut s = self.sender.borrow().clone();
            s.connection = ConnectionState::Connected;
            self.sender.send_replace(s);
            Ok(())
        }
        async fn disconnect(&mut self) -> Result<(), DriverError> {
            let mut s = self.sender.borrow().clone();
            s.connection = ConnectionState::Disconnected {
                reason: "test".into(),
            };
            self.sender.send_replace(s);
            Ok(())
        }
        fn status(&self) -> PrinterStatus {
            self.sender.borrow().clone()
        }
        fn subscribe_status(&self) -> watch::Receiver<PrinterStatus> {
            self.receiver.clone()
        }
        async fn send(
            &self,
            _: SendPayload,
            _on_progress: UploadProgressFn,
        ) -> Result<SendHandle, DriverError> {
            Err(DriverError::NotConnected)
        }
        async fn command(&self, _: PrinterCommand) -> Result<(), DriverError> {
            Err(DriverError::NotConnected)
        }
    }

    #[test]
    fn register_assigns_monotonic_ids() {
        let reg = DriverRegistry::new();
        let id1 = reg.register(Box::new(StubDriver::new(DriverKind::Bambu)));
        let id2 = reg.register(Box::new(StubDriver::new(DriverKind::U1)));
        assert_ne!(id1, id2);
        assert!(id2.0 > id1.0);
    }

    #[test]
    fn get_returns_some_for_registered_some_for_unknown() {
        let reg = DriverRegistry::new();
        let id = reg.register(Box::new(StubDriver::new(DriverKind::Bambu)));
        assert!(reg.get(id).is_some());
        assert!(reg.get(DriverId(9999)).is_none());
    }

    #[test]
    fn remove_returns_true_for_present_false_for_missing() {
        let reg = DriverRegistry::new();
        let id = reg.register(Box::new(StubDriver::new(DriverKind::Bambu)));
        assert!(reg.remove(id));
        assert!(!reg.remove(id));
    }
}
