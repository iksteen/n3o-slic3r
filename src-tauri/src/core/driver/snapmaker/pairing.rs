//! The Snapmaker U1 LAN pairing dance on the cleartext MQTT broker
//! (`:1884`).
//!
//! The printer accepts an unauthenticated MQTT CONNECT and serves a
//! constant `12345678/config/{request,response,notification}` channel
//! gated by an on-screen approval popup. On approval it returns per-client
//! TLS material (`ca`, `cert`, `key`, `sn`, `port`) on the notification
//! channel — that becomes the persisted [`SnapToken`].
//!
//! Faithful port of `iksteen/machin3d-overlay`'s `moonraker/u1/pair.rs`.
//! The protocol (topic names, the `confirm_lan_status` →
//! `request_lan_auth` → `notify_lan_auth` sequence) is verified there
//! against real hardware.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rumqttc::{AsyncClient, ConnectReturnCode, Event, EventLoop, MqttOptions, Packet, QoS};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::snap_token::SnapToken;
use crate::core::driver::traits::DriverError;

/// The cleartext bootstrap broker port.
pub const PAIRING_PORT: u16 = 1884;

/// The bootstrap channel is gated by a constant string every client
/// shares — the printer authorizes by an on-screen tap, not by knowledge
/// of this value. Topics under this prefix are the only ones the cleartext
/// `:1884` broker accepts publishes on.
const CONFIG_REQUEST_TOPIC: &str = "12345678/config/request";
const CONFIG_RESPONSE_TOPIC: &str = "12345678/config/response";
const CONFIG_NOTIFICATION_TOPIC: &str = "12345678/config/notification";
const KEEPALIVE: Duration = Duration::from_secs(30);

/// Generate a fresh client identifier of the shape the printer expects.
/// Persisted in the [`SnapToken`] so a re-pair against the same printer
/// reuses it and skips the on-screen approval.
pub fn fresh_clientid() -> String {
    format!("n3o-{}", Uuid::new_v4())
}

/// Run the pairing dance against `host`, waiting up to `approval_timeout`
/// for the user to tap Approve. `clientid` is the stable identifier the
/// printer keys its auth DB on — pass a previously-stored one to re-pair
/// without re-tapping, or [`fresh_clientid`] for a first pairing.
pub async fn pair(
    host: &str,
    clientid: &str,
    approval_timeout: Duration,
) -> Result<SnapToken, DriverError> {
    let bootstrap_clientid = format!("n3o-try-{}", unix_millis());
    let mut options = MqttOptions::new(bootstrap_clientid, host.to_owned(), PAIRING_PORT);
    options.set_keep_alive(KEEPALIVE);
    options.set_clean_session(false);

    let (client, eventloop) = AsyncClient::new(options, 32);

    let token = tokio::time::timeout(
        approval_timeout,
        run_bootstrap(client.clone(), eventloop, host, clientid),
    )
    .await
    .map_err(|_| {
        DriverError::Network(format!(
            "timed out after {}s waiting for the printer's approval popup; tap Approve on the printer and retry",
            approval_timeout.as_secs()
        ))
    })?;

    let _ = client.disconnect().await;
    token
}

async fn run_bootstrap(
    client: AsyncClient,
    mut eventloop: EventLoop,
    host: &str,
    clientid: &str,
) -> Result<SnapToken, DriverError> {
    subscribe_bootstrap_topics(&client, host).await?;
    publish_confirm(&client, clientid).await?;

    let app_id = format!("n3o-{}", unix_millis());
    let mut sent_auth_request = false;

    loop {
        let event = eventloop
            .poll()
            .await
            .map_err(|e| DriverError::Network(format!("pairing MQTT loop failed: {e}")))?;
        match event {
            Event::Incoming(Packet::ConnAck(ack)) if ack.code != ConnectReturnCode::Success => {
                return Err(DriverError::Network(format!(
                    "printer rejected cleartext MQTT CONNECT: {:?}",
                    ack.code
                )));
            }
            Event::Incoming(Packet::Publish(publish)) => match publish.topic.as_str() {
                CONFIG_RESPONSE_TOPIC => {
                    let body: ConfigResponse =
                        serde_json::from_slice(&publish.payload).map_err(|e| {
                            DriverError::Protocol(format!("bad config-response payload: {e}"))
                        })?;
                    match body.result.state.as_str() {
                        "reject" | "rejected" => {
                            return Err(DriverError::Auth(format!(
                                "printer rejected the pairing request: {}",
                                body.result.message.unwrap_or_default()
                            )));
                        }
                        state => {
                            // Any other state (`unauthorized` for a new
                            // clientid, `authorizing` while the popup is up,
                            // `success` on a bare ack) means "drive
                            // request_lan_auth and wait for the
                            // notification". It's idempotent, so once.
                            if !sent_auth_request {
                                sent_auth_request = true;
                                tracing::info!(
                                    state,
                                    "U1 pairing: requesting authorization — tap Approve on the printer"
                                );
                                publish_request_auth(&client, clientid, &app_id).await?;
                            }
                        }
                    }
                }
                CONFIG_NOTIFICATION_TOPIC => {
                    let body: ConfigNotification =
                        serde_json::from_slice(&publish.payload).map_err(|e| {
                            DriverError::Protocol(format!("bad config-notification payload: {e}"))
                        })?;
                    if body.method != "notify_lan_auth" {
                        continue;
                    }
                    let entry = body
                        .params
                        .into_iter()
                        .find(|entry| entry.clientid == clientid)
                        .ok_or_else(|| {
                            DriverError::Protocol(
                                "notify_lan_auth did not include our clientid".to_owned(),
                            )
                        })?;
                    if entry.state != "approve" {
                        return Err(DriverError::Auth(format!(
                            "printer authorization ended with state `{}`",
                            entry.state
                        )));
                    }
                    return Ok(token_from_notification(host, clientid, entry));
                }
                _ => {}
            },
            _ => {}
        }
    }
}

async fn subscribe_bootstrap_topics(client: &AsyncClient, host: &str) -> Result<(), DriverError> {
    for topic in [
        format!("{host}/status"),
        CONFIG_RESPONSE_TOPIC.to_owned(),
        format!("{host}/notification"),
        CONFIG_NOTIFICATION_TOPIC.to_owned(),
    ] {
        client
            .subscribe(topic.clone(), QoS::AtLeastOnce)
            .await
            .map_err(|e| DriverError::Network(format!("subscribe {topic}: {e}")))?;
    }
    Ok(())
}

async fn publish_confirm(client: &AsyncClient, clientid: &str) -> Result<(), DriverError> {
    let body = json!({
        "jsonrpc": "2.0",
        "method": "server.client_manager.confirm_lan_status",
        "params": {"clientid": clientid},
        "id": unix_millis(),
    });
    publish_request(client, &body, "confirm_lan_status").await
}

async fn publish_request_auth(
    client: &AsyncClient,
    clientid: &str,
    app_id: &str,
) -> Result<(), DriverError> {
    let body = json!({
        "jsonrpc": "2.0",
        "method": "server.client_manager.request_lan_auth",
        "params": {"clientid": clientid, "app_id": app_id},
        "id": unix_millis(),
    });
    publish_request(client, &body, "request_lan_auth").await
}

async fn publish_request(
    client: &AsyncClient,
    body: &serde_json::Value,
    label: &str,
) -> Result<(), DriverError> {
    let payload = serde_json::to_vec(body)
        .map_err(|e| DriverError::Other(format!("encode {label}: {e}")))?;
    client
        .publish(CONFIG_REQUEST_TOPIC, QoS::AtLeastOnce, false, payload)
        .await
        .map_err(|e| DriverError::Network(format!("publish {label}: {e}")))
}

fn token_from_notification(host: &str, clientid: &str, entry: NotifyAuthEntry) -> SnapToken {
    SnapToken {
        host: host.to_owned(),
        sn: entry.sn,
        clientid: clientid.to_owned(),
        mqtt_port: entry.port,
        ca: entry.ca,
        cert: entry.cert,
        key: entry.key,
    }
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[derive(Debug, Deserialize)]
struct ConfigResponse {
    result: ConfigResult,
}

#[derive(Debug, Deserialize)]
struct ConfigResult {
    state: String,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfigNotification {
    method: String,
    #[serde(default)]
    params: Vec<NotifyAuthEntry>,
}

#[derive(Debug, Deserialize)]
struct NotifyAuthEntry {
    state: String,
    clientid: String,
    sn: String,
    ca: String,
    cert: String,
    key: String,
    port: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unauthorized_and_authorizing_responses() {
        for state in ["unauthorized", "authorizing"] {
            let json = format!(
                r#"{{"jsonrpc":"2.0","result":{{"state":"{state}","clientid":"x","message":"m"}}}}"#
            );
            let body: ConfigResponse = serde_json::from_str(&json).unwrap();
            assert_eq!(body.result.state, state);
        }
    }

    #[test]
    fn parses_notify_lan_auth_into_token() {
        let json = r#"{"jsonrpc":"2.0","method":"notify_lan_auth","params":[{"state":"approve","clientid":"n3o-x","sn":"SN1","ca":"CA","cert":"CERT","key":"KEY","port":8883,"app_id":"a"}]}"#;
        let body: ConfigNotification = serde_json::from_str(json).unwrap();
        assert_eq!(body.method, "notify_lan_auth");
        let entry = body.params.into_iter().next().unwrap();
        let token = token_from_notification("192.168.0.120", "n3o-x", entry);
        assert_eq!(token.host, "192.168.0.120");
        assert_eq!(token.sn, "SN1");
        assert_eq!(token.clientid, "n3o-x");
        assert_eq!(token.mqtt_port, 8883);
        assert_eq!(token.key, "KEY");
    }

    #[test]
    fn fresh_clientid_is_unique_and_prefixed() {
        let a = fresh_clientid();
        let b = fresh_clientid();
        assert!(a.starts_with("n3o-"));
        assert_ne!(a, b);
    }
}
