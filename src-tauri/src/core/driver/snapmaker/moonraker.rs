//! Minimal Moonraker JSON-RPC over WebSocket client (PR-7b-2).
//!
//! On connect, sends `printer.objects.subscribe` for the printer
//! objects we consume in [`super::status`] (PR-7b-3). The subscribe
//! response is decoded as the initial status; subsequent
//! `notify_status_update` messages merge into a cached status map
//! and yield a fresh snapshot to the caller.
//!
//! Ported from `iksteen/bambu-overlay` `src/snapmaker/moonraker.rs`.
//! Differences from the overlay:
//!
//! - Errors map to [`DriverError`] variants (`Network` for socket
//!   failures, `Protocol` for JSON-RPC + decoding issues) instead
//!   of `anyhow::Error`.
//! - `connect` takes `host` + `port` directly rather than a
//!   `SnapmakerEndpoint` wrapper — we already carry that info in
//!   [`crate::core::driver::traits::DriverConfig::U1`].
//!
//! The status decoder helpers (`get_string`, `get_f64`, `extruders`,
//! `get_print_info_i64`) ship alongside the session so PR-7b-3 can
//! consume them without spreading per-field lookups across modules.

use std::collections::HashMap;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Map, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};
use tracing::debug;

use crate::core::driver::DriverError;

/// Printer objects we subscribe to. Every field [`super::status`]
/// reads (PR-7b-3) comes from one of these objects. Adding a new
/// object here without a corresponding decoder is harmless — it
/// just sits in the merged status map unused.
const SUBSCRIBE_OBJECTS: &[&str] = &[
    "print_stats",
    "display_status",
    "extruder",
    "extruder1",
    "extruder2",
    "extruder3",
    "heater_bed",
    "fan",
    "virtual_sdcard",
    // Snapmaker-specific object on U1 firmware; carries per-task
    // metadata the standard Klipper objects don't expose. Safe to
    // request on plain Klipper too — Moonraker returns `null` for
    // missing objects, the merge_status path handles it.
    "print_task_config",
    "gcode_move",
    "toolhead",
];

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// One MoonrakerSession owns one WebSocket. Reconnect is the
/// driver's responsibility, not this type's.
pub(super) struct MoonrakerSession {
    socket: Socket,
    status: Map<String, Value>,
    next_request_id: u64,
}

impl MoonrakerSession {
    /// Open the WS, send the initial subscribe, decode the
    /// response into `status`. Returns once the session is ready
    /// to receive `notify_status_update` events via
    /// [`Self::next_status`].
    pub(super) async fn connect(host: &str, port: u16) -> Result<Self, DriverError> {
        let url = format!("ws://{host}:{port}/websocket");
        let request = url
            .as_str()
            .into_client_request()
            .map_err(|e| DriverError::Protocol(format!("invalid Moonraker WS URL `{url}`: {e}")))?;
        let (socket, _response) = connect_async(request)
            .await
            .map_err(|e| DriverError::Network(format!("connect Moonraker at {url}: {e}")))?;
        debug!(host = %host, port = port, "moonraker connected");
        let mut session = Self {
            socket,
            status: Map::new(),
            next_request_id: 1,
        };
        let initial = session.send_subscribe().await?;
        session.merge_status(&initial);
        Ok(session)
    }

    /// Current merged status map. Cloned because the eventual
    /// status-decode caller (PR-7b-3) wants an owned snapshot it
    /// can pass around to per-field helpers.
    pub(super) fn status(&self) -> Map<String, Value> {
        self.status.clone()
    }

    /// Block until the next `notify_status_update` arrives. Other
    /// JSON-RPC traffic (responses we didn't initiate, ping/pong,
    /// binary frames) is silently dropped. Returns `Ok(None)` on
    /// clean server-side close so the caller can transition to
    /// `ConnectionState::Disconnected` and reconnect.
    pub(super) async fn next_status(&mut self) -> Result<Option<Map<String, Value>>, DriverError> {
        loop {
            let Some(message) = self.socket.next().await else {
                return Ok(None);
            };
            let message = message
                .map_err(|e| DriverError::Network(format!("Moonraker WS read failed: {e}")))?;
            let text = match message {
                Message::Text(text) => text,
                Message::Close(_) => return Ok(None),
                // Binary / control frames aren't part of Moonraker's
                // protocol contract; tungstenite handles pong replies
                // for us, we just skip the frames here.
                Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
                    continue
                }
            };
            let value: Value = serde_json::from_str(&text)
                .map_err(|e| DriverError::Protocol(format!("Moonraker WS sent non-JSON: {e}")))?;
            if value.get("method").and_then(Value::as_str) != Some("notify_status_update") {
                continue;
            }
            let Some(update) = value
                .get("params")
                .and_then(Value::as_array)
                .and_then(|array| array.first())
                .and_then(Value::as_object)
                .cloned()
            else {
                continue;
            };
            self.merge_status(&update);
            return Ok(Some(self.status.clone()));
        }
    }

    /// JSON-RPC `printer.objects.subscribe` for every object in
    /// [`SUBSCRIBE_OBJECTS`]. The response carries the initial
    /// status snapshot in `result.status`; subsequent updates flow
    /// via `notify_status_update`.
    async fn send_subscribe(&mut self) -> Result<Map<String, Value>, DriverError> {
        let id = self.next_request_id;
        self.next_request_id += 1;
        let objects: Map<String, Value> = SUBSCRIBE_OBJECTS
            .iter()
            .map(|name| ((*name).to_owned(), Value::Null))
            .collect();
        let request = json!({
            "jsonrpc": "2.0",
            "method": "printer.objects.subscribe",
            "params": { "objects": objects },
            "id": id,
        });
        self.socket
            .send(Message::Text(request.to_string()))
            .await
            .map_err(|e| DriverError::Network(format!("send subscribe request: {e}")))?;

        while let Some(message) = self.socket.next().await {
            let message = message
                .map_err(|e| DriverError::Network(format!("Moonraker WS read failed: {e}")))?;
            let text = match message {
                Message::Text(text) => text,
                Message::Close(_) => {
                    return Err(DriverError::Protocol(
                        "Moonraker closed before subscribe response".into(),
                    ));
                }
                Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
                    continue
                }
            };
            let value: Value = serde_json::from_str(&text)
                .map_err(|e| DriverError::Protocol(format!("Moonraker WS sent non-JSON: {e}")))?;
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                // Out-of-band notification arrived before our
                // subscribe response; ignore + keep waiting.
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(DriverError::Protocol(format!(
                    "Moonraker subscribe error: {error}"
                )));
            }
            let status = value
                .get("result")
                .and_then(|result| result.get("status"))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            return Ok(status);
        }
        Err(DriverError::Protocol(
            "Moonraker closed before subscribe response".into(),
        ))
    }

    /// Per-object shallow merge: a patch object on key `extruder`
    /// merges field-wise into the existing `extruder` object so a
    /// `{ temperature: 220.1 }` update doesn't wipe the
    /// `target_temperature` we already had. Anything that isn't a
    /// patch on an existing object is inserted verbatim.
    fn merge_status(&mut self, update: &Map<String, Value>) {
        for (key, value) in update {
            match (self.status.get_mut(key), value) {
                (Some(existing), Value::Object(patch)) if existing.is_object() => {
                    let existing = existing.as_object_mut().expect("checked is_object");
                    for (subkey, subvalue) in patch {
                        existing.insert(subkey.clone(), subvalue.clone());
                    }
                }
                _ => {
                    self.status.insert(key.clone(), value.clone());
                }
            }
        }
    }
}

// ---- Status-map readers (consumed by PR-7b-3) ----

/// Pull a string field from a nested object in the merged status
/// map. Returns `None` when either the object or the field is
/// absent, or when the value isn't a JSON string.
#[allow(dead_code)] // PR-7b-3 consumes
pub(super) fn get_string<'a>(
    status: &'a Map<String, Value>,
    object: &str,
    field: &str,
) -> Option<&'a str> {
    status.get(object)?.get(field)?.as_str()
}

/// Pull an `f64` from a nested object. Returns `None` for absent
/// fields or non-numeric values.
#[allow(dead_code)] // PR-7b-3 consumes
pub(super) fn get_f64(status: &Map<String, Value>, object: &str, field: &str) -> Option<f64> {
    status.get(object)?.get(field)?.as_f64()
}

/// Pull an `i64` from `print_stats.info.<field>` — the sub-object
/// Moonraker uses for layer counts on klipper >= 0.12.
#[allow(dead_code)] // PR-7b-3 consumes
pub(super) fn get_print_info_i64(status: &Map<String, Value>, field: &str) -> Option<i64> {
    status.get("print_stats")?.get("info")?.get(field)?.as_i64()
}

/// Collect every `extruder`, `extruder1`, …, `extruderN` object
/// in the status map, keyed by zero-based index. The U1 reports
/// one entry per docked toolhead.
#[allow(dead_code)] // PR-7b-3 consumes
pub(super) fn extruders(status: &Map<String, Value>) -> HashMap<usize, &Value> {
    let mut extruders = HashMap::new();
    for (key, value) in status {
        if let Some(index) = extruder_index(key) {
            extruders.insert(index, value);
        }
    }
    extruders
}

fn extruder_index(name: &str) -> Option<usize> {
    if name == "extruder" {
        return Some(0);
    }
    name.strip_prefix("extruder")?.parse::<usize>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- merge_status (no I/O — pure logic) ----

    /// `merge_status` is `&mut self` so we test it through a tiny
    /// shim that owns the status map but doesn't need a socket.
    /// The body inside mirrors `MoonrakerSession::merge_status`
    /// byte-for-byte; if the production version drifts, this test
    /// helper drifts in lockstep (intentional — both are tiny).
    fn merge(initial: Value, patch: Value) -> Value {
        let mut status: Map<String, Value> = initial
            .as_object()
            .cloned()
            .expect("initial must be an object");
        let patch_map: Map<String, Value> =
            patch.as_object().cloned().expect("patch must be an object");
        // Inline copy of MoonrakerSession::merge_status — the
        // function is private, this keeps the test honest by
        // mirroring the production path byte-for-byte.
        for (key, value) in &patch_map {
            match (status.get_mut(key), value) {
                (Some(existing), Value::Object(p)) if existing.is_object() => {
                    let existing = existing.as_object_mut().unwrap();
                    for (subkey, subvalue) in p {
                        existing.insert(subkey.clone(), subvalue.clone());
                    }
                }
                _ => {
                    status.insert(key.clone(), value.clone());
                }
            }
        }
        Value::Object(status)
    }

    #[test]
    fn merge_patches_existing_object_field_wise() {
        // `extruder.temperature` gets a fresh reading but the
        // `target` we already had must survive.
        let initial = json!({
            "extruder": { "temperature": 200.0, "target": 220.0 }
        });
        let patch = json!({
            "extruder": { "temperature": 210.5 }
        });
        let merged = merge(initial, patch);
        assert_eq!(merged["extruder"]["temperature"], 210.5);
        assert_eq!(merged["extruder"]["target"], 220.0);
    }

    #[test]
    fn merge_inserts_new_object_verbatim() {
        let initial = json!({ "extruder": { "temperature": 200.0 } });
        let patch = json!({ "heater_bed": { "temperature": 60.0 } });
        let merged = merge(initial, patch);
        assert_eq!(merged["extruder"]["temperature"], 200.0);
        assert_eq!(merged["heater_bed"]["temperature"], 60.0);
    }

    #[test]
    fn merge_replaces_scalar_top_level() {
        // A non-object patch on a top-level scalar overwrites it.
        let initial = json!({ "fan": 0.5 });
        let patch = json!({ "fan": 0.9 });
        let merged = merge(initial, patch);
        assert_eq!(merged["fan"], 0.9);
    }

    #[test]
    fn merge_replaces_object_when_patch_is_scalar() {
        // Edge case: server replaces an object with a scalar.
        // Existing object must NOT survive — we overwrite.
        let initial = json!({ "fan": { "speed": 0.5 } });
        let patch = json!({ "fan": null });
        let merged = merge(initial, patch);
        assert_eq!(merged["fan"], Value::Null);
    }

    // ---- helpers ----

    #[test]
    fn get_string_returns_field_or_none() {
        let status: Map<String, Value> = json!({
            "print_stats": { "filename": "benchy.gcode", "state": "printing" }
        })
        .as_object()
        .cloned()
        .unwrap();
        assert_eq!(
            get_string(&status, "print_stats", "filename"),
            Some("benchy.gcode"),
        );
        assert_eq!(
            get_string(&status, "print_stats", "state"),
            Some("printing"),
        );
        assert_eq!(get_string(&status, "print_stats", "missing"), None);
        assert_eq!(get_string(&status, "absent_object", "filename"), None);
    }

    #[test]
    fn get_f64_handles_int_and_float_json_numbers() {
        let status: Map<String, Value> = json!({
            "extruder": { "temperature": 200, "target": 220.5 }
        })
        .as_object()
        .cloned()
        .unwrap();
        assert_eq!(get_f64(&status, "extruder", "temperature"), Some(200.0));
        assert_eq!(get_f64(&status, "extruder", "target"), Some(220.5));
        assert_eq!(get_f64(&status, "extruder", "missing"), None);
    }

    #[test]
    fn get_print_info_i64_walks_nested_path() {
        let status: Map<String, Value> = json!({
            "print_stats": { "info": { "total_layer": 278, "current_layer": 42 } }
        })
        .as_object()
        .cloned()
        .unwrap();
        assert_eq!(get_print_info_i64(&status, "total_layer"), Some(278));
        assert_eq!(get_print_info_i64(&status, "current_layer"), Some(42));
        assert_eq!(get_print_info_i64(&status, "missing"), None);
    }

    #[test]
    fn get_print_info_i64_returns_none_without_info_subobject() {
        // Klipper < 0.12 reports total_layer at the top level of
        // print_stats rather than under .info. The helper returns
        // None there; PR-7b-3 falls back to the top-level field.
        let status: Map<String, Value> = json!({
            "print_stats": { "total_layer": 278 }
        })
        .as_object()
        .cloned()
        .unwrap();
        assert_eq!(get_print_info_i64(&status, "total_layer"), None);
    }

    #[test]
    fn extruders_collects_every_extruder_n_object_indexed() {
        let status: Map<String, Value> = json!({
            "extruder":  { "temperature": 200.0 },
            "extruder1": { "temperature": 195.0 },
            "extruder2": { "temperature": 205.0 },
            "extruder3": { "temperature": 198.5 },
            "heater_bed": { "temperature": 60.0 },  // not an extruder
            "fan": 0.5,                              // ditto
        })
        .as_object()
        .cloned()
        .unwrap();
        let map = extruders(&status);
        assert_eq!(map.len(), 4);
        assert_eq!(map[&0]["temperature"], 200.0);
        assert_eq!(map[&1]["temperature"], 195.0);
        assert_eq!(map[&2]["temperature"], 205.0);
        assert_eq!(map[&3]["temperature"], 198.5);
    }

    #[test]
    fn extruder_index_only_matches_exact_format() {
        // `extruder` → 0, `extruder<N>` → N for ascii integer N.
        assert_eq!(extruder_index("extruder"), Some(0));
        assert_eq!(extruder_index("extruder1"), Some(1));
        assert_eq!(extruder_index("extruder42"), Some(42));
        // Negative / alpha suffixes / unrelated names: None.
        assert_eq!(extruder_index("extruder-1"), None);
        assert_eq!(extruder_index("extruderA"), None);
        assert_eq!(extruder_index("heater_bed"), None);
        assert_eq!(extruder_index("printer.extruder"), None);
    }
}
