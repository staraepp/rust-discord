#![allow(dead_code)]

use std::time::Duration;

use dc_gateway::session::{Session, SessionConfig};
use dc_protocol::Secret;
use dc_protocol::gateway::ClientProperties;
use serde_json::Value;

/// Obviously fake; must never appear in logs or Debug output.
pub const FAKE_TOKEN: &str = "fake-token-for-tests-only";

pub const HELLO: &str = fixture!("gateway/hello.json");
pub const HEARTBEAT_ACK: &str = fixture!("gateway/heartbeat_ack.json");
pub const HEARTBEAT_REQUEST: &str = fixture!("gateway/heartbeat_request.json");
pub const RECONNECT: &str = fixture!("gateway/reconnect.json");
pub const INVALID_SESSION: &str = fixture!("gateway/invalid_session.json");
pub const INVALID_SESSION_RESUMABLE: &str = fixture!("gateway/invalid_session_resumable.json");
pub const READY: &str = fixture!("gateway/ready.json");
pub const RESUMED: &str = fixture!("gateway/resumed.json");
pub const MESSAGE_CREATE: &str = fixture!("gateway/message_create.json");
pub const UNKNOWN_EVENT: &str = fixture!("gateway/unknown_event.json");

/// Heartbeat interval in the Hello fixture.
pub const INTERVAL: Duration = Duration::from_millis(41_250);

pub const BASE_URL: &str = "wss://gateway.example.invalid";
pub const BASE_WS: &str = "wss://gateway.example.invalid/?v=9&encoding=json";
pub const RESUME_WS: &str = "wss://gateway-resume.example.invalid/?v=9&encoding=json";

pub fn config() -> SessionConfig {
    SessionConfig::new(ClientProperties {
        os: "linux".into(),
        browser: "dc-test".into(),
        device: "dc-test".into(),
    })
}

/// A session with a fixed jitter value, so every delay is exact.
pub fn session(jitter: f64) -> Session {
    Session::new(
        Secret::new(FAKE_TOKEN.to_owned()),
        config(),
        Box::new(move || jitter),
    )
}

pub fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("valid json")
}
