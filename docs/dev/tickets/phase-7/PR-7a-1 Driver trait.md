# PR-7a-1 — `Driver` trait + `DriverRegistry`

Status: ❌ open.

**Scope.** Common abstraction that both drivers (Bambu MQTT,
U1 Moonraker) implement. Drives the Tauri command shape — one
`driver_*` command set, dispatched via the registry by
`driver_id`. Pre-emptively shaped for Phase 8's plugin-host
takeover so the trait doesn't get redesigned twice.

**Acceptance criteria.**

- New module `core/driver/`:
  - `mod.rs` re-exports.
  - `traits.rs` — the `Driver` trait + supporting domain types
    (`DriverId`, `DriverError`, `SendPayload`, `SendHandle`,
    `PrinterCommand`).
  - `status.rs` — `PrinterStatus` (common fields) + `DriverExtra`
    (typed enum, one variant per driver).
  - `registry.rs` — `DriverRegistry` (`HashMap<DriverId, Arc<Mutex<Box<dyn Driver>>>>`)
    with `register`, `get`, `remove`, `list`.

- `Driver` trait shape (async via `async-trait`):

  ```rust
  #[async_trait]
  pub trait Driver: Send + Sync {
      fn id(&self) -> &DriverId;
      fn kind(&self) -> DriverKind;            // Bambu / U1
      async fn connect(&mut self) -> Result<(), DriverError>;
      async fn disconnect(&mut self) -> Result<(), DriverError>;
      fn status(&self) -> PrinterStatus;
      fn subscribe_status(&self) -> watch::Receiver<PrinterStatus>;
      async fn send(&mut self, payload: SendPayload) -> Result<SendHandle, DriverError>;
      async fn command(&mut self, cmd: PrinterCommand) -> Result<(), DriverError>;
  }
  ```

- `SendPayload`:
  ```rust
  pub enum SendPayload {
      /// Bambu A1 mini: pre-built .gcode.3mf bundle (from PR-3-10
      /// + PR-7c-7's binding metadata).
      Gcode3mf { bytes: Vec<u8>, plate_id: u32 },
      /// Snapmaker U1: raw G-code body.
      Gcode { bytes: Vec<u8>, file_name: String },
  }
  ```

- `PrinterCommand`:
  ```rust
  pub enum PrinterCommand {
      Pause,
      Resume,
      Stop,
  }
  ```

- `PrinterStatus` (common fields all drivers fill):
  ```rust
  pub struct PrinterStatus {
      pub connection: ConnectionState,         // Connecting / Connected / Reconnecting / Disconnected(reason)
      pub job: Option<JobProgress>,
      pub temps: Temps,
      pub extra: DriverExtra,                  // BambuExtra | U1Extra
      pub last_updated: SystemTime,
  }
  ```

- `JobProgress`: `file_name`, `current_layer`, `total_layers`,
  `percent`, `eta_seconds`, `state` (Idle / Printing / Paused /
  Finished / Failed{reason}).

- `Temps`: `nozzles: Vec<TempReading>`, `bed: TempReading`,
  `chamber: Option<TempReading>` (per-toolhead for U1, single
  for A1 mini).

- `DriverExtra` enum:
  ```rust
  pub enum DriverExtra {
      Bambu(BambuExtra),    // AMS slot states, mounted plate, fan
      U1(U1Extra),          // mounted toolhead, per-toolhead filament, fan
  }
  ```
  Empty placeholders shipped in PR-7a-1; populated by their
  respective driver tickets.

- `DriverError`:
  ```rust
  pub enum DriverError {
      Network(String),
      Auth(String),
      Protocol(String),
      Cancelled,
      NotConnected,
      Other(String),
  }
  ```
  `Display` impl yields a user-facing message.

- `DriverRegistry` stored as `Tauri::State<Arc<Mutex<DriverRegistry>>>`
  alongside the existing `CascadeRegistry` / `JobRegistry`.

- Tauri command surface (in `core/driver/commands.rs`):
  - `driver_register(kind: DriverKind, config: DriverConfig) -> Result<DriverId, String>`
  - `driver_unregister(id: DriverId) -> Result<(), String>`
  - `driver_list() -> Vec<DriverSummary>` — id, kind, connection state, status snapshot.
  - `driver_connect(id: DriverId) -> Result<(), String>`
  - `driver_disconnect(id: DriverId) -> Result<(), String>`
  - `driver_status(id: DriverId) -> Result<PrinterStatus, String>`
  - `driver_send(id: DriverId, payload: SendPayload) -> Result<SendHandle, String>`
  - `driver_command(id: DriverId, cmd: PrinterCommand) -> Result<(), String>`

- Tauri event surface:
  - `driver:status_update` — emitted ≤1 Hz per driver, payload
    `{ driver_id, status }`. Throttled at the driver task; see
    PR-7a-3 for the throttle implementation.

- Tests:
  - `Driver` trait object-safety check (sanity unit test that
    constructs a `Box<dyn Driver>` from a stub impl).
  - `DriverRegistry` register/get/remove/list happy path.
  - `PrinterCommand` + `SendPayload` serde round-trip — they
    cross the Tauri boundary.

**Effort.** ~1 day. Pure scaffolding; no protocol work. Pays
off when 7a/7b/7c land against the contract.

**Dependencies.** None — first ticket of Phase 7.

**Out of scope.**

- Stub driver implementation — that's PR-7a-2 (Bambu) and
  PR-7b-2 (U1) building against this trait.
- Plugin-host integration — Phase 8 lifts the trait into a
  plugin boundary; the shape needs to be plugin-friendly but
  the actual plugin host is not Phase 7 work.
