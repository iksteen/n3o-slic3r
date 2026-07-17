//! Snapmaker U1 status over the printer's mTLS MQTT bus.
//!
//! A paired U1's status transport. Speaks the same
//! `printer.objects.subscribe` / `notify_status_update` protocol as the
//! generic Moonraker WebSocket session — and reuses that module's decode
//! helpers — but carries it over the printer's mTLS MQTT control plane
//! instead. That's what makes status work remotely: off-LAN the WebSocket
//! isn't a trusted client, but a valid client cert still reaches the
//! broker. An unpaired/local printer uses the WebSocket transport.
//!
//! Implements the vendor-agnostic
//! [`StatusSession`](crate::core::driver::moonraker::StatusSession) so the
//! generic Moonraker driver drives it without knowing the transport;
//! [`MqttSessionFactory`] is injected at driver construction.
//!
//! Topics are `<sn>/{request,response,status}` (SN = the paired token's
//! serial): publish `printer.objects.subscribe` on `…/request`, read the
//! snapshot reply on `…/response`, then merge the streamed
//! `notify_status_update` frames on `…/status`. Verified against a live
//! printer.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rumqttc::{AsyncClient, ConnectReturnCode, Event, EventLoop, MqttOptions, Packet, QoS};
use serde_json::{json, Map, Value};

use super::mtls;
use super::snap_token::SnapToken;
use crate::core::driver::moonraker::session::{merge_status_into, SUBSCRIBE_OBJECTS};
use crate::core::driver::moonraker::{StatusSession, StatusSessionFactory};
use crate::core::driver::traits::ControlPlane;
use crate::core::driver::DriverError;

const MQTT_KEEPALIVE: Duration = Duration::from_secs(30);
/// rumqttc request-channel capacity — we only ever publish the one
/// subscribe request, so this is slack, not a tuning knob.
const CHANNEL_CAP: usize = 32;

/// Opens the U1 mTLS MQTT status transport for a paired printer. Injected
/// into the Moonraker driver in place of the WebSocket factory.
pub struct MqttSessionFactory {
    pub token: SnapToken,
}

#[async_trait]
impl StatusSessionFactory for MqttSessionFactory {
    async fn connect(&self) -> Result<Box<dyn StatusSession>, DriverError> {
        MqttStatusSession::connect(&self.token)
            .await
            .map(|session| Box::new(session) as Box<dyn StatusSession>)
    }
}

/// One status session over the mTLS MQTT bus. Owns its rumqttc event
/// loop and polls it directly in `next_status`, the same single-consumer
/// shape as the WebSocket session — reconnect is the driver worker's job,
/// not this type's.
struct MqttStatusSession {
    client: AsyncClient,
    eventloop: EventLoop,
    status: Map<String, Value>,
    request_topic: String,
    status_topic: String,
    response_topic: String,
    next_request_id: u64,
}

impl MqttStatusSession {
    /// Connect, subscribe to the status + response topics, send the
    /// initial `printer.objects.subscribe`, and decode its reply into
    /// `status`. The caller bounds this with a connect timeout, so the
    /// internal waits loop without their own deadline.
    async fn connect(token: &SnapToken) -> Result<Self, DriverError> {
        let sn = token.sn.as_str();
        // Distinct client id from the camera's monitor session: an MQTT
        // broker evicts an existing connection when a second joins with the
        // same id, so sharing `token.clientid` makes camera and status kick
        // each other in a loop. The broker's ACL is cert-based, so the id
        // value is free — only its uniqueness matters.
        let mut options = MqttOptions::new(
            format!("{}-status", token.clientid),
            token.host.clone(),
            token.mqtt_port,
        );
        options.set_keep_alive(MQTT_KEEPALIVE);
        options.set_clean_session(true);
        options.set_transport(mtls::transport_for(token)?);

        let (client, eventloop) = AsyncClient::new(options, CHANNEL_CAP);
        let mut session = Self {
            client,
            eventloop,
            status: Map::new(),
            request_topic: format!("{sn}/request"),
            status_topic: format!("{sn}/status"),
            response_topic: format!("{sn}/response"),
            next_request_id: 1,
        };

        for topic in [&session.status_topic, &session.response_topic] {
            session
                .client
                .subscribe(topic.clone(), QoS::AtMostOnce)
                .await
                .map_err(|e| DriverError::Network(format!("subscribe {topic}: {e}")))?;
        }

        session.wait_for_connack().await?;
        session.send_subscribe().await?;
        Ok(session)
    }

    async fn wait_for_connack(&mut self) -> Result<(), DriverError> {
        loop {
            let event = self
                .eventloop
                .poll()
                .await
                .map_err(|e| DriverError::Network(format!("U1 MQTT connect: {e}")))?;
            if let Event::Incoming(Packet::ConnAck(ack)) = event {
                if ack.code != ConnectReturnCode::Success {
                    return Err(DriverError::Auth(format!(
                        "U1 MQTT CONNECT rejected: {:?}",
                        ack.code
                    )));
                }
                return Ok(());
            }
        }
    }

    /// Publish `printer.objects.subscribe` and drive the event loop until
    /// its id-matched reply lands on the response topic, merging the
    /// `result.status` snapshot. Any `notify_status_update` that races
    /// ahead of the reply is merged too, so no early frame is lost.
    async fn send_subscribe(&mut self) -> Result<(), DriverError> {
        let id = self.next_request_id;
        self.next_request_id += 1;
        let objects: Map<String, Value> = SUBSCRIBE_OBJECTS
            .iter()
            .map(|name| ((*name).to_owned(), Value::Null))
            .collect();
        let payload = json!({
            "jsonrpc": "2.0",
            "method": "printer.objects.subscribe",
            "params": {"objects": objects},
            "id": id,
        });
        self.client
            .publish(
                self.request_topic.clone(),
                QoS::AtLeastOnce,
                false,
                serde_json::to_vec(&payload).expect("serialize subscribe request"),
            )
            .await
            .map_err(|e| DriverError::Network(format!("publish subscribe request: {e}")))?;

        loop {
            let event = self
                .eventloop
                .poll()
                .await
                .map_err(|e| DriverError::Network(format!("U1 MQTT subscribe: {e}")))?;
            let Event::Incoming(Packet::Publish(publish)) = event else {
                continue;
            };
            if publish.topic == self.status_topic {
                if let Some(update) = status_update_params(&publish.payload) {
                    merge_status_into(&mut self.status, &update);
                }
            } else if publish.topic == self.response_topic {
                let value: Value = serde_json::from_slice(&publish.payload)
                    .map_err(|e| DriverError::Protocol(format!("U1 MQTT non-JSON reply: {e}")))?;
                if value.get("id").and_then(Value::as_u64) != Some(id) {
                    continue;
                }
                if let Some(error) = value.get("error") {
                    return Err(DriverError::Protocol(format!(
                        "U1 MQTT subscribe error: {error}"
                    )));
                }
                if let Some(status) = value
                    .get("result")
                    .and_then(|result| result.get("status"))
                    .and_then(Value::as_object)
                {
                    merge_status_into(&mut self.status, status);
                }
                return Ok(());
            }
        }
    }
}

/// Fire-and-forget JSON-RPC publishes on `<sn>/request`. Clones of the
/// rumqttc client share the session's connection; sends fail once its
/// event loop is gone.
struct MqttControl {
    client: AsyncClient,
    request_topic: String,
}

#[async_trait]
impl ControlPlane for MqttControl {
    async fn send_jsonrpc(&self, method: &str, params: Value) -> Result<(), DriverError> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": jsonrpc_id(),
        });
        self.client
            .publish(
                self.request_topic.clone(),
                QoS::AtLeastOnce,
                false,
                serde_json::to_vec(&payload).expect("serialize jsonrpc request"),
            )
            .await
            .map_err(|e| DriverError::Network(format!("MQTT publish {method}: {e}")))
    }
}

/// Request ids for fire-and-forget sends — unix millis, unique enough for
/// requests whose replies we never read.
fn jsonrpc_id() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[async_trait]
impl StatusSession for MqttStatusSession {
    fn status(&self) -> Map<String, Value> {
        self.status.clone()
    }

    /// Block until the next `notify_status_update` on the status topic,
    /// merge it, and yield a fresh snapshot. `Ok(None)` on a clean broker
    /// disconnect so the worker reconnects; `Err` on a transport failure.
    async fn next_status(&mut self) -> Result<Option<Map<String, Value>>, DriverError> {
        loop {
            let event = self
                .eventloop
                .poll()
                .await
                .map_err(|e| DriverError::Network(format!("U1 MQTT status poll: {e}")))?;
            match event {
                Event::Incoming(Packet::Publish(publish)) if publish.topic == self.status_topic => {
                    if let Some(update) = status_update_params(&publish.payload) {
                        merge_status_into(&mut self.status, &update);
                        return Ok(Some(self.status.clone()));
                    }
                }
                Event::Incoming(Packet::Disconnect) => return Ok(None),
                _ => {}
            }
        }
    }

    fn control(&self) -> Arc<dyn ControlPlane> {
        Arc::new(MqttControl {
            client: self.client.clone(),
            request_topic: self.request_topic.clone(),
        })
    }
}

/// Extract the object patch from a `notify_status_update` frame —
/// `params[0]`. Returns `None` for any other message so the caller skips
/// it. Shared shape with the WebSocket session's `next_status`.
fn status_update_params(payload: &[u8]) -> Option<Map<String, Value>> {
    let value: Value = serde_json::from_slice(payload).ok()?;
    if value.get("method").and_then(Value::as_str) != Some("notify_status_update") {
        return None;
    }
    value
        .get("params")
        .and_then(Value::as_array)
        .and_then(|array| array.first())
        .and_then(Value::as_object)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_update_params_extracts_the_patch() {
        let frame = json!({
            "jsonrpc": "2.0",
            "method": "notify_status_update",
            "params": [{"extruder": {"temperature": 221.0}}, 1974975.4],
        })
        .to_string();
        let patch = status_update_params(frame.as_bytes()).expect("patch");
        assert_eq!(patch["extruder"]["temperature"], 221.0);
    }

    #[test]
    fn status_update_params_ignores_other_methods() {
        let reply = json!({"jsonrpc": "2.0", "id": 1, "result": {"status": {}}}).to_string();
        assert!(status_update_params(reply.as_bytes()).is_none());
        assert!(status_update_params(b"not json").is_none());
    }
}
