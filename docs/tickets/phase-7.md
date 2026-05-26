# Phase 7 — tickets

Phase 7 (printer connectivity + filament sync, ~6 person-weeks
across three sub-phases) is the **printers-actually-work phase**
— the PRD's exit criterion that "both printers receive jobs from
the app and complete prints successfully without manual G-code
editing." Source: `docs/Execution_Plan.md` §9. Stated goal:

> Send-and-monitor for both MVP printers, plus filament sync and
> material-binding UX. Highest-risk phase for surprises. Filament
> sync is a major UX investment that materially differentiates
> the product.

Phase 7 is **three thin vertical slices stacked** — each sub-
phase ends with a real print:

- **Phase 7a** (Bambu A1 mini, ~2 weeks): LAN MQTT driver,
  `.gcode.3mf` send, status + AMS lite read, pause/resume/stop.
  Closes the gap from Phase 3's offline-only slice and Phase 6's
  preview to "click Slice, click Send, watch the printer print."
- **Phase 7b** (Snapmaker U1, ~2 weeks): HTTP driver, U1 cascade,
  toolchanger G-code validation, plain `.gcode` send. The U1 is
  Klipper-based with a 4-toolhead carriage; this is where the
  per-toolhead cascade scope from Phase 1 finally exits the
  laboratory.
- **Phase 7c** (filament sync, ~2 weeks): per-slot loaded-
  filament identity polling, project material binding UX,
  mismatch detection, sync-on-send. The cross-cutting UX layer
  that makes "load the same project on either printer and get
  the right colors" actually true.

**Sequencing.** Strictly sequential: 7a fully → 7b → 7c. Each
sub-phase ends with a real-print smoke gate on physical hardware
(both available); landing 7a end-to-end de-risks the driver-
abstraction shape that 7b conforms to and that 7c relies on. No
parallelism — wall-clock cost (~6 weeks) accepted in exchange for
each milestone being independently shippable.

Individual tickets live one-per-file in `phase-7/`. This file is
the index plus phase-level status and notes.

## Status by deliverable

### 7a — Bambu A1 mini (~2 weeks)

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| `Driver` trait + `DriverRegistry` (shared with 7b) | ❌ open | [PR-7a-1](phase-7/PR-7a-1%20Driver%20trait.md) |
| rumqttc MQTT connection + auth + status subscribe | ❌ open | [PR-7a-2](phase-7/PR-7a-2%20Bambu%20MQTT%20connection.md) |
| Status parser → `PrinterStatus` + mounted build plate read | ❌ open | [PR-7a-3](phase-7/PR-7a-3%20Bambu%20status%20parser.md) |
| AMS lite state read (slot 1–4 filament identity) | ❌ open | [PR-7a-4](phase-7/PR-7a-4%20AMS%20lite%20state.md) |
| `.gcode.3mf` upload + send-print command | ❌ open | [PR-7a-5](phase-7/PR-7a-5%20Bambu%20send.md) |
| Pause / resume / stop commands | ❌ open | [PR-7a-6](phase-7/PR-7a-6%20Bambu%20commands.md) |
| Frontend printer state panel + send button | ❌ open | [PR-7a-7](phase-7/PR-7a-7%20Bambu%20panel.md) |
| Real-print smoke + walkthrough doc | ❌ open | [PR-7a-8](phase-7/PR-7a-8%20Bambu%20smoke.md) |

### 7b — Snapmaker U1 (~2 weeks)

The U1 is Klipper-based and exposes **standard Moonraker** over
WebSocket at `ws://HOST:PORT/websocket`, not a Snapmaker-specific
HTTP wrapper as the PRD originally assumed. The user's
`bambu-overlay` reference implementation confirms this (see
`src/snapmaker/moonraker.rs:49-58`). The mTLS MQTT control plane
exists too but is camera-only and out of scope for MVP send +
status.

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| U1 cascade TOML (4 toolheads, plates, start/end gcode) | ✅ done | [PR-7b-1](phase-7/PR-7b-1%20U1%20cascade.md) |
| Moonraker WebSocket client + JSON-RPC + status subscribe | ✅ done | [PR-7b-2](phase-7/PR-7b-2%20U1%20HTTP%20client.md) |
| Status parser → mounted toolhead + per-toolhead state | ✅ done | [PR-7b-3](phase-7/PR-7b-3%20U1%20status%20parser.md) |
| Plain `.gcode` upload (multipart) + start-print | ✅ done | [PR-7b-4](phase-7/PR-7b-4%20U1%20send.md) |
| Toolchanger G-code emission validation | ✅ done | [PR-7b-5](phase-7/PR-7b-5%20Toolchanger%20gcode.md) |
| Pause / resume / stop commands | ✅ done | [PR-7b-6](phase-7/PR-7b-6%20U1%20commands.md) |
| Per-toolhead independent nozzle/hotend cascade wiring | ✅ done | [PR-7b-7](phase-7/PR-7b-7%20Per-toolhead%20cascade.md) |
| Frontend U1 state panel | ❌ open | [PR-7b-8](phase-7/PR-7b-8%20U1%20panel.md) |
| Real-print smoke set (1/2/4-mat + tool-change stress) | 🟡 partial | [PR-7b-9](phase-7/PR-7b-9%20U1%20smoke.md) |

### 7c — Filament sync + assignment (~2 weeks)

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| Filament profile library + cascade integration | ❌ open | [PR-7c-1](phase-7/PR-7c-1%20Filament%20library.md) |
| `FilamentState` per-printer model + driver hookup | ❌ open | [PR-7c-2](phase-7/PR-7c-2%20Filament%20state%20model.md) |
| Filament state panel UI + manual override + badge | ❌ open | [PR-7c-3](phase-7/PR-7c-3%20Filament%20state%20UI.md) |
| Mismatch detector (family / temp ±10°C / color) + warn-vs-block | ❌ open | [PR-7c-4](phase-7/PR-7c-4%20Mismatch%20detector.md) |
| Auto-binding heuristic (family match on first assignment) | ❌ open | [PR-7c-5](phase-7/PR-7c-5%20Auto-binding.md) |
| Per-(plate, printer) binding persistence | ❌ open | [PR-7c-6](phase-7/PR-7c-6%20Per-plate-printer%20bindings.md) |
| Sync-on-send (per-driver metadata emission) | ❌ open | [PR-7c-7](phase-7/PR-7c-7%20Sync%20on%20send.md) |
| Multi-color paint UI binds to material indices | ❌ open | [PR-7c-8](phase-7/PR-7c-8%20Paint%20to%20material%20index.md) |
| Phase 7 exit-criteria smoke + walkthrough doc | ❌ open | [PR-7c-9](phase-7/PR-7c-9%20Exit-criteria%20smoke.md) |

## Architecture invariant — drivers live behind one trait

The two printer drivers (Bambu MQTT, U1 HTTP) share zero
protocol but the same lifecycle, the same status surface, and
the same command surface. PR-7a-1's `Driver` trait pins that
contract:

```rust
#[async_trait]
pub trait Driver: Send + Sync {
    fn id(&self) -> DriverId;
    async fn connect(&mut self) -> Result<(), DriverError>;
    async fn disconnect(&mut self) -> Result<(), DriverError>;
    fn status(&self) -> &PrinterStatus;
    fn subscribe_status(&self) -> watch::Receiver<PrinterStatus>;
    async fn send(&mut self, payload: SendPayload) -> Result<SendHandle, DriverError>;
    async fn command(&mut self, cmd: PrinterCommand) -> Result<(), DriverError>;
}
```

`SendPayload` is an enum that lets each driver consume what its
firmware expects:

```rust
pub enum SendPayload {
    /// Bambu A1 mini: .gcode.3mf bundle (PR-3-10 writer +
    /// PR-7c-7 binding metadata).
    Gcode3mf(Vec<u8>),
    /// Snapmaker U1: plain G-code bytes.
    Gcode(Vec<u8>),
}
```

`PrinterStatus` is union-shaped: every driver fills the common
fields (state, current layer, temps); driver-specific extras
live in a typed `extra: DriverExtra` field with one variant per
driver. Frontend reads common fields generically + branches on
`extra` for AMS / toolhead detail.

**Why a trait now.** Phase 8's plugin system will lift drivers
into plugins. The trait shape needs to be the same one a plugin
implements, or we re-architect twice. Designing for the plugin
interface up front costs ~half a day and saves the rewrite.

**Resist:** per-driver Tauri commands like
`bambu_send_print(...)` / `u1_send_print(...)`. The Tauri
boundary takes `(driver_id, payload)` and dispatches via the
registry. Driver-specific UX surfaces (AMS lite panel, U1
toolhead strip) are component-level branches over `extra`, not
backend boundaries.

## Architecture invariant — driver tasks own their connection lifecycle

Each driver runs on a dedicated Tokio task that owns the
connection (MQTT client / HTTP polling loop). The trait methods
above are message-passing wrappers over channels into that task,
not direct protocol calls. Reasons:

- MQTT is event-driven (incoming status messages arrive
  asynchronously); HTTP polling has its own tick. Both want to
  push status updates into a shared `watch::Sender<PrinterStatus>`
  without callers blocking.
- Reconnect-on-disconnect is the driver task's problem, not the
  caller's. UI commands queue against the task; the task
  surfaces "I'm reconnecting" via the status stream.
- Phase 8's plugin model isolates each driver in its own task /
  process anyway — getting the channel shape right now eases
  that transition.

## What's *not* in Phase 7

- **Cloud-API printers.** Bambu Cloud, Bambu Handy, BambuLab
  account-based control — out. LAN-only per the PRD.
- **OctoPrint / Moonraker / generic Klipper drivers.** U1 is
  Snapmaker-flavored Klipper accessed via Snapmaker's own HTTP
  wrapper. Generic Klipper drivers are Phase 8 plugin material.
- **Webcam streams.** Bambu has a webcam; U1 has one too. Not
  in MVP — separate panel/process effort, post-MVP.
- **Print queue / multi-job scheduling.** One job at a time per
  printer. Queue UX is Phase 9 polish.
- **Sliced-by-someone-else send.** The send button operates on
  the active plate's own slice. A "send any .gcode file"
  affordance is Phase 9.
- **Filament catalog imports from BBS/Orca.** Custom profile
  editor lives in cascade (PR-7c-1); importing from upstream
  catalogs in bulk is post-MVP.

## Dependency graph

```
PR-7a-1 (Driver trait + registry)
  ├── PR-7a-2 (rumqttc connect + status sub)
  │    ├── PR-7a-3 (status parser + mounted plate)
  │    ├── PR-7a-4 (AMS lite state)
  │    ├── PR-7a-5 (.gcode.3mf send)
  │    └── PR-7a-6 (pause/resume/stop)
  └── (consumed by 7b — see below)

PR-7a-1..-6 + Phase 5 plate state
  └── PR-7a-7 (frontend Bambu panel)

PR-7a-1..-7
  └── PR-7a-8 (Bambu real-print smoke)  ← 7a exit

# --- 7a → 7b handoff ---

PR-7a-1 (Driver trait, now battle-tested by 7a)
  ├── PR-7b-1 (U1 cascade)
  ├── PR-7b-2 (HTTP client)
  │    ├── PR-7b-3 (status parser)
  │    ├── PR-7b-4 (plain .gcode send)
  │    └── PR-7b-6 (pause/resume/stop)
  └── PR-7b-7 (per-toolhead cascade wiring) ← depends on PR-7b-1

PR-7b-1 + libslic3r FFI
  └── PR-7b-5 (toolchanger gcode validation)

PR-7b-1..-7
  └── PR-7b-8 (frontend U1 panel)

PR-7b-1..-8
  └── PR-7b-9 (U1 real-print smoke set)  ← 7b exit

# --- 7b → 7c handoff ---

Both drivers (PR-7a + PR-7b) + Phase 5 binding model
  ├── PR-7c-1 (filament profile library)
  ├── PR-7c-2 (FilamentState model + driver hookup)
  │    ├── PR-7c-3 (state UI + override)
  │    ├── PR-7c-4 (mismatch detector)
  │    ├── PR-7c-5 (auto-binding)
  │    └── PR-7c-6 (per-(plate,printer) persistence)
  └── PR-7c-7 (sync-on-send) ← depends on PR-7c-2 + PR-7a-5 + PR-7b-4

PR-7c-1..-7
  ├── PR-7c-8 (paint UI → material indices)
  └── PR-7c-9 (Phase 7 exit smoke)  ← phase exit
```

The critical path is **PR-7a-1 → PR-7a-2 → PR-7a-5 → PR-7a-8**:
landing one Bambu print end-to-end. Everything that comes after
is incremental on top of a known-working pipeline. Spike 3 has
already proved the metadata format works; this critical path
adds the network + send + status loop.

## Exit criteria for the phase (from Execution Plan §9)

- Both printers receive jobs from the app and complete prints
  successfully without manual G-code editing.
- Status of both printers can be monitored simultaneously.
- Filament sync works: changing what's loaded in a printer is
  reflected in the app within one poll cycle; mismatches are
  caught before slice.
- A multi-color project sliced for either printer assigns model
  materials to the correct physical slots and prints with the
  expected colors.

PR-7c-9 mechanizes these as a real-hardware-required walkthrough
in `docs/phase-7-smoke.md`; the per-sub-phase smokes (PR-7a-8 +
PR-7b-9) gate the individual driver exits.

## Cut candidates (from Execution Plan §9)

User signed off on **none** at planning time — full scope. Trim
only if the phase runs hot:

- **AMS lite filament identity read** (keep slot count) — saves
  3 days; degrades multi-color UX. Strong candidate to keep.
- **Pause / resume / stop commands** (send-only) — saves 2 days
  per driver. User pauses from the printer's own UI.
- **Auto-binding heuristic** (manual binding only) — saves 2 days.
- **Mismatch detection beyond family** (skip ±10°C check) —
  saves 1 day.
- **Manual filament-identity override** (printer-reported only)
  — saves 2 days. Hurts third-party-spool users.

## Implementation notes

### Reference implementation — `iksteen/bambu-overlay`

The user's own working overlay project covers status/connect for
both printers verbatim. **Read-only side is solved**; Phase 7
tickets cite specific source paths there. The write side
(send-print, pause/resume/stop) is NOT in bambu-overlay — those
flows reference BambuStudio source + Moonraker docs.

Coverage map:

| Concern | bambu-overlay path | Phase 7 ticket |
|---|---|---|
| Bambu rumqttc + native-tls + custom CA | `src/device_tls.rs:14-71`, `src/mqtt/target.rs:107-117` | PR-7a-2 |
| Bambu device-id probe (peer cert CN) | `src/local/device.rs:13-37` | PR-7a-2 |
| Bambu status subscribe + pushall request | `src/mqtt/session.rs:39-67,113-122` | PR-7a-3 |
| Bambu AMS payload shape | `src/bambu/{models.rs:181-200,report.rs:38-75}` | PR-7a-4 |
| Bambu FTPS shape (download — adapt for upload) | `src/thumbnail/local.rs:132-160` | PR-7a-5 |
| U1 Moonraker WebSocket + JSON-RPC | `src/snapmaker/moonraker.rs:25-58` | PR-7b-2 |
| U1 serial probe | `src/snapmaker/probe.rs:21-56` | PR-7b-2 |
| U1 status object list + merge | `src/snapmaker/moonraker.rs:25-39,162-176` | PR-7b-3 |
| U1 state-string mapping | `src/snapmaker/report.rs:57-69` | PR-7b-3 |
| U1 per-toolhead filament | `src/snapmaker/report.rs:108-145` | PR-7b-3 |

### MQTT crate: rumqttc + native-tls

Bambu's LAN MQTT uses a self-signed certificate with a serial-
number-as-CN that doesn't match the printer's IP. bambu-overlay
uses **`native-tls` (not `rustls`)** because suppaftp 8.0.3
can't expose the peer cert post-handshake with rustls, and BBL's
device certs are X.509 v1 which rules out rustls' custom verifier
path. PR-7a-2 inherits this choice for symmetry.

Connector recipe (verbatim from `src/device_tls.rs:60-71`):
disable built-in roots, add the embedded BBL CA chain,
`danger_accept_invalid_hostnames(true)`. The embedded BBL CA is
hard-coded PEM in `src/device_tls.rs:14-37` — **expires
2032-04-01**, flagged in code comment.

Auth: user `"bblp"`, password = LAN access code (shown on
printer LCD under network settings), port 8883. Device-id is the
serial number, but bambu-overlay reads it from the peer cert CN
via a probe TLS connection rather than asking the user — PR-7a-2
adopts the same pattern.

### .gcode.3mf upload protocol — FTPS

Bambu uploads via FTPS implicit-TLS on port 990, login
`bblp` + access code, passive mode + `set_passive_nat_workaround(true)`.
bambu-overlay's `connect_local_ftps` (`src/thumbnail/local.rs:132-160`)
is the connection shape — change `RETR` to `PUT` for upload.

Note: post-handshake, the FTPS hostname check is skipped (BBL
device certs are X.509 v1 — same reason native-tls is used).
After upload, publish an MQTT command pointing at the uploaded
file path. **bambu-overlay does not implement the upload + MQTT
command pair** — PR-7a-5 sources the command shape from
BambuStudio's `src/slic3r/Utils/PrintHost.cpp` flow.

### U1 Moonraker WebSocket

**Correction to PRD §10.B.6.** The U1 exposes standard Moonraker
JSON-RPC, not Snapmaker's own HTTP wrapper. Endpoint:
`ws://HOST:PORT/websocket` (default port 80). No auth on the
status side; the mTLS bootstrap exists only for camera
control, which is post-MVP.

Probe + status flow (verbatim from
`src/snapmaker/{probe.rs,moonraker.rs}`):
1. `GET http://HOST:PORT/machine/system_info` →
   `.result.system_info.product_info.serial_number` is the
   device id.
2. WebSocket connect → JSON-RPC
   `printer.objects.subscribe` with this object list:
   `print_stats, display_status, extruder, extruder1..3,
   heater_bed, fan, virtual_sdcard, print_task_config,
   gcode_move, toolhead`.
3. Printer pushes `notify_status_update` with deltas; merge into
   cached map (`merge_status` at `moonraker.rs:162-176`).

Send-print not in bambu-overlay — Moonraker docs:
`POST /server/files/upload` (multipart) → `POST /printer/print/start`.
Pause/resume/stop: `POST /printer/print/{pause,resume,cancel}`.

### Per-toolhead cascade — already in PrinterProfile

PR-1-7's `PrinterProfile` already has `Vec<Toolhead>` with
per-slot nozzle/hotend fields. PR-7b-7 is the cascade-side wiring
to expose per-slot overrides through the settings panel
(`when.slot = N` predicates already exist from PR-1-2).

### Filament profile library scope (PR-7c-1)

MVP profile set:
- Bambu Generic PLA / PETG / ABS (3 profiles, mirror what BBS
  ships)
- "Generic PLA / PETG / ABS" (3 profiles, brand-agnostic
  defaults)

That's 6 bundled profiles. Custom profile editor extends the
cascade via the same TOML pattern PR-1-8 used for filaments.

### Sync-on-send formats

- **Bambu**: AMS bindings emitted into `Metadata/plate_N.json`'s
  `filament_settings_id` + `nozzle_diameter` arrays. Already
  partially structured by PR-3-10; PR-7c-7 wires the per-plate
  binding into that emission.
- **U1**: Per-toolhead filament identity emitted as G-code
  header comments (`; filament_settings_id = …`). The U1
  firmware reads these to validate the loaded filament matches.

## Open questions seeded for the implementer

- **Bambu send-print MQTT command shape.** bambu-overlay is
  read-only; the upload + print-start MQTT command pair must
  come from BambuStudio source (`BambuStudio/src/slic3r/Utils/PrintHost.cpp`).
  PR-7a-5 ticks the recipe.
- **AMS lite vs full AMS topology** (PR-7a-4). The A1 mini
  ships AMS lite (4 external spools, no buffer). bambu-overlay
  reads the same `print.ams.ams[].tray[]` shape regardless of
  topology — the differentiator is which trays are populated.
  Full AMS (16 slots, 4 buffers) detection is "is this an A1
  mini or X1C" and out of scope for MVP.
- **U1 build plate reporting** (PR-7b-3). The Moonraker object
  list bambu-overlay subscribes to doesn't include a build
  plate identifier — Snapmaker may report it as a custom
  Klipper printer object. Falls back to manual selection from
  the U1 cascade's plate list if no auto-report is found.
- **Reconnect backoff strategy.** Both drivers need exponential
  backoff on disconnect. PR-7a-2 ticks the design; PR-7b-2
  mirrors it. Open whether to expose backoff state to the UI
  ("Reconnecting in 4s…") or just log + silent retry. The
  bambu-overlay `supervisor.rs` has a reference implementation.
- **Status event firehose vs throttle** (PR-7a-3 / PR-7b-3).
  Bambu MQTT can fire 5+ status messages per second during
  printing; Moonraker similar with the wide object list.
  Decide on rate-limiting at the driver task (collapse to ~1 Hz
  for UI, preserve all events for trace logging).
- **Per-printer connection persistence.** Storing the
  `(host, access_code, serial)` triple per printer in Project /
  per-printer-in-project (so opening a project on a new machine
  asks for credentials)? Or a global app-state file? The PRD
  implies per-printer-in-project. PR-7a-7's settings UX needs
  a decision; bambu-overlay's `src/local/config.rs` shape
  is a reference.
- **AMS slot mismatch with paint UI** (PR-7c-8). If the user
  has 4 paint colors but the AMS only has 3 spools loaded, the
  fourth color must surface as a mismatch + still produce a
  sliceable result (e.g., by mapping all unmapped colors to
  slot 1 with a visible warning). PR-7c-4 + PR-7c-8 share this
  edge case — coordinate.
- **BBL CA expiry handling.** The embedded BBL CA in
  `device_tls.rs:18` expires 2032-04-01. Phase 7a should
  surface a build-time warning when the build date is within
  6 months of the CA expiry. Tracked separately from this
  phase but flagged here so it doesn't get lost.
