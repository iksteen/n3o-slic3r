# PR-7a-2 — Bambu rumqttc connection + auth + status subscribe

Status: ❌ open.

**Scope.** First half of the Bambu driver: spin up rumqttc with
the right TLS config, perform the device-id probe, subscribe to
the status topic, request the initial `pushall` snapshot, hand
incoming messages off for parsing. No status interpretation (PR-7a-3)
or AMS (PR-7a-4) yet — just the connection lifecycle and raw
message intake.

The user's `bambu-overlay` reference implementation has this
solved. Mirror it.

**Acceptance criteria.**

- New module `core/driver/bambu/`:
  - `mod.rs` re-exports `BambuDriver`.
  - `tls.rs` — TLS connector setup.
  - `device_id.rs` — peer-cert-CN probe.
  - `connection.rs` — rumqttc lifecycle (connect, subscribe,
    pushall, reconnect loop).
  - `BambuDriver` struct implementing the `Driver` trait
    (PR-7a-1).

- **Add deps** to `src-tauri/Cargo.toml`:
  - `rumqttc = "0.24"` (or current stable).
  - `native-tls` (NOT rustls — see index doc Implementation
    Notes for rationale).
  - `tokio` (already present; ensure full feature flag).

- **TLS connector** (`tls.rs`), mirroring
  `bambu-overlay/src/device_tls.rs:14-71`:
  - Embed Bambu's CA chain as PEM (copy from bambu-overlay
    verbatim; comment notes expiry 2032-04-01).
  - Build a `native_tls::TlsConnector` with
    `disable_built_in_roots(true)`, add the embedded CA,
    `danger_accept_invalid_hostnames(true)` (device cert CN
    is the serial, not the host).

- **Device-id probe** (`device_id.rs`), mirroring
  `bambu-overlay/src/local/device.rs:13-37`:
  - Open a raw TLS socket to `printer:8883` with the connector
    above.
  - Read the peer cert's subject CN.
  - Return the CN as the device id (serial number).
  - Runs once at driver `connect()` time; cached for the
    session.

- **Connection lifecycle** (`connection.rs`):
  - `rumqttc::AsyncClient` with `MqttOptions`:
    - host + port 8883
    - credentials user `"bblp"` / password = LAN access code
    - keep-alive 60s (mirror bambu-overlay `target.rs:15`)
    - TLS transport from above
  - On `Event::Incoming(Packet::ConnAck)`:
    - Subscribe to `device/<DEVICE_ID>/report`, QoS 0.
    - Publish a pushall request to `device/<DEVICE_ID>/request`:
      ```json
      {"pushing":{"sequence_id":"0","command":"pushall","version":1,"push_target":1}}
      ```
      (mirror `bambu-overlay/src/mqtt/session.rs:113-122`).
  - On `Event::Incoming(Packet::Publish)`:
    - Forward payload bytes to a `mpsc::Sender<BambuMessage>`
      channel for PR-7a-3's parser.
  - On disconnect: exponential backoff (1s, 2s, 4s, 8s, 16s,
    cap 60s), surface `ConnectionState::Reconnecting { in_seconds }`
    via the `watch::Sender<PrinterStatus>` from PR-7a-1.

- **Driver trait impl**:
  - `connect()`: device-id probe → spawn the rumqttc event loop
    task → wait for first ConnAck → return Ok.
  - `disconnect()`: signal the event loop task to exit cleanly;
    timeout 5s.
  - `subscribe_status()`: returns a clone of the `watch::Receiver`.
  - `status()`: clones the latest `PrinterStatus` from the
    `watch::Sender` borrowed.
  - `send()` / `command()`: stub `Err(DriverError::NotConnected)`
    for this ticket — implemented in PR-7a-5 / PR-7a-6.

- **Configuration**:
  ```rust
  pub struct BambuConfig {
      pub host: String,           // IP or hostname
      pub access_code: String,    // 8-char code from printer LCD
      pub serial: Option<String>, // optional — probe overrides
  }
  ```

- Tests:
  - **`tls_connector_disables_builtin_roots`** — construct the
    connector, assert it rejects a cert signed by Let's Encrypt
    and accepts one signed by the embedded BBL CA. Static
    test cert fixtures in `tests/fixtures/bambu-tls/`.
  - **`device_id_probe_returns_cn_from_peer_cert`** — spawn a
    tiny stub TLS server with a CN-as-serial cert, point the
    probe at it, assert it returns the CN.
  - **`reconnect_backoff_caps_at_60s`** — drive the connection
    state machine with N synthetic failures, assert delay
    sequence matches.
  - **No real-printer test in this ticket** — that's PR-7a-8.

**Effort.** ~2 days. Half of it is staring at bambu-overlay's
TLS dance to make sure we copy it faithfully (native-tls vs
rustls, the hostname check disable, the CA expiry comment).

**Dependencies.** PR-7a-1.

**Out of scope.**

- Status interpretation — PR-7a-3.
- AMS payload parsing — PR-7a-4.
- File upload + send-print — PR-7a-5.
- Pause / resume / stop — PR-7a-6.
- Multi-printer connection management — single driver instance
  per printer; the registry from PR-7a-1 handles multiplexing.
