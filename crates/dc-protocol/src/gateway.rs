//! Gateway payloads: opcodes, the envelope, send events and receive events.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{Guild, Message, MessageDelete, MessageUpdate, Ready};
use crate::secret::{Secret, serialize_exposed};
use crate::{API_VERSION, ProtocolError};

macro_rules! opcodes {
    ($($(#[$doc:meta])* $name:ident = $value:literal,)+) => {
        /// Gateway opcodes (userdoccers "Gateway Opcodes" table).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum Opcode {
            $($(#[$doc])* $name = $value,)+
        }

        impl Opcode {
            pub const fn from_u8(value: u8) -> Option<Self> {
                match value {
                    $($value => Some(Self::$name),)+
                    _ => None,
                }
            }

            pub const fn as_u8(self) -> u8 {
                self as u8
            }
        }
    };
}

opcodes! {
    /// Receive: an event was dispatched.
    Dispatch = 0,
    /// Send/Receive: keep the connection alive.
    Heartbeat = 1,
    /// Send: start a new session.
    Identify = 2,
    PresenceUpdate = 3,
    VoiceStateUpdate = 4,
    VoiceServerPing = 5,
    /// Send: resume a disconnected session.
    Resume = 6,
    /// Receive: reconnect and resume immediately.
    Reconnect = 7,
    RequestGuildMembers = 8,
    /// Receive: the session has been invalidated.
    InvalidSession = 9,
    /// Receive: sent after connecting; carries the heartbeat interval.
    Hello = 10,
    /// Receive: acknowledges a heartbeat.
    HeartbeatAck = 11,
    GuildSync = 12,
    CallConnect = 13,
    GuildSubscriptions = 14,
    LobbyConnect = 15,
    LobbyDisconnect = 16,
    LobbyVoiceStates = 17,
    StreamCreate = 18,
    StreamDelete = 19,
    StreamWatch = 20,
    StreamPing = 21,
    StreamSetPaused = 22,
    LfgSubscriptions = 23,
    RequestGuildApplicationCommands = 24,
    EmbeddedActivityCreate = 25,
    EmbeddedActivityDelete = 26,
    EmbeddedActivityUpdate = 27,
    RequestForumUnreads = 28,
    RemoteCommand = 29,
    RequestDeletedEntityIds = 30,
    RequestSoundboardSounds = 31,
    SpeedTestCreate = 32,
    SpeedTestDelete = 33,
    RequestLastMessages = 34,
    SearchRecentMembers = 35,
    RequestChannelStatuses = 36,
    GuildSubscriptionsBulk = 37,
    GuildChannelsResync = 38,
    RequestChannelMemberCount = 39,
    QosHeartbeat = 40,
    UpdateTimeSpentSessionId = 41,
    LobbyVoiceServerPing = 42,
    RequestChannelInfo = 43,
}

/// Builds the WebSocket URL for a gateway base URL (from `GET /gateway` or
/// READY's `resume_gateway_url`).
///
/// Transport compression is not requested in M0.
pub fn gateway_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}/?v={API_VERSION}&encoding=json")
}

// ---------------------------------------------------------------------------
// Receive
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RawPayload {
    op: u8,
    #[serde(default)]
    d: Value,
    #[serde(default)]
    s: Option<u64>,
    #[serde(default)]
    t: Option<String>,
}

/// Opcode 10 Hello data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct Hello {
    /// Heartbeat interval in milliseconds.
    pub heartbeat_interval: u64,
}

/// A decoded payload received from the gateway.
#[derive(Clone, Debug, PartialEq)]
pub enum GatewayReceive {
    /// Opcode 0. `data` is decoded lazily with [`DispatchEvent::decode`].
    Dispatch {
        seq: Option<u64>,
        name: String,
        data: Value,
    },
    /// Opcode 1: the gateway asks for an immediate heartbeat.
    Heartbeat,
    /// Opcode 7.
    Reconnect,
    /// Opcode 9. `resumable` is the `d` boolean.
    InvalidSession { resumable: bool },
    /// Opcode 10.
    Hello(Hello),
    /// Opcode 11.
    HeartbeatAck,
    /// An opcode this client does not handle. Ignored, never fatal.
    Unknown { op: u8 },
}

impl GatewayReceive {
    /// Decodes one text frame.
    pub fn parse(text: &str) -> Result<Self, ProtocolError> {
        let raw: RawPayload =
            serde_json::from_str(text).map_err(ProtocolError::MalformedPayload)?;
        Ok(match Opcode::from_u8(raw.op) {
            Some(Opcode::Dispatch) => GatewayReceive::Dispatch {
                seq: raw.s,
                name: raw.t.unwrap_or_default(),
                data: raw.d,
            },
            Some(Opcode::Heartbeat) => GatewayReceive::Heartbeat,
            Some(Opcode::Reconnect) => GatewayReceive::Reconnect,
            Some(Opcode::InvalidSession) => GatewayReceive::InvalidSession {
                // Anything that is not literally `true` is not resumable.
                resumable: raw.d.as_bool().unwrap_or(false),
            },
            Some(Opcode::Hello) => GatewayReceive::Hello(
                serde_json::from_value(raw.d).map_err(ProtocolError::MalformedPayload)?,
            ),
            Some(Opcode::HeartbeatAck) => GatewayReceive::HeartbeatAck,
            _ => GatewayReceive::Unknown { op: raw.op },
        })
    }
}

/// A typed dispatch (opcode 0) event.
#[derive(Clone, Debug, PartialEq)]
pub enum DispatchEvent {
    Ready(Box<Ready>),
    Resumed,
    MessageCreate(Box<Message>),
    MessageUpdate(Box<MessageUpdate>),
    MessageDelete(MessageDelete),
    GuildCreate(Box<Guild>),
    /// Any event not modelled yet, passed through untouched.
    Unknown {
        name: String,
        data: Value,
    },
}

impl DispatchEvent {
    /// Decodes the `d` of a dispatch by event name.
    pub fn decode(name: &str, data: Value) -> Result<Self, ProtocolError> {
        fn typed<T: serde::de::DeserializeOwned>(
            name: &str,
            data: Value,
        ) -> Result<T, ProtocolError> {
            serde_json::from_value(data).map_err(|source| ProtocolError::MalformedEvent {
                event: name.to_owned(),
                source,
            })
        }

        Ok(match name {
            "READY" => DispatchEvent::Ready(Box::new(typed(name, data)?)),
            "RESUMED" => DispatchEvent::Resumed,
            "MESSAGE_CREATE" => DispatchEvent::MessageCreate(Box::new(typed(name, data)?)),
            "MESSAGE_UPDATE" => DispatchEvent::MessageUpdate(Box::new(typed(name, data)?)),
            "MESSAGE_DELETE" => DispatchEvent::MessageDelete(typed(name, data)?),
            "GUILD_CREATE" => DispatchEvent::GuildCreate(Box::new(typed(name, data)?)),
            _ => DispatchEvent::Unknown {
                name: name.to_owned(),
                data,
            },
        })
    }

    /// The gateway event name.
    pub fn name(&self) -> &str {
        match self {
            DispatchEvent::Ready(_) => "READY",
            DispatchEvent::Resumed => "RESUMED",
            DispatchEvent::MessageCreate(_) => "MESSAGE_CREATE",
            DispatchEvent::MessageUpdate(_) => "MESSAGE_UPDATE",
            DispatchEvent::MessageDelete(_) => "MESSAGE_DELETE",
            DispatchEvent::GuildCreate(_) => "GUILD_CREATE",
            DispatchEvent::Unknown { name, .. } => name,
        }
    }
}

// ---------------------------------------------------------------------------
// Send
// ---------------------------------------------------------------------------

/// Client properties sent with Identify.
///
/// These describe *this* client truthfully. The docs note that "no field is
/// truly required"; we deliberately do not imitate an official client's
/// properties or build numbers (CLAUDE.md hard rule 2: no fingerprint
/// spoofing).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientProperties {
    pub os: String,
    pub browser: String,
    pub device: String,
}

/// Initial presence sent with Identify.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IdentifyPresence {
    pub activities: Vec<Value>,
    pub status: String,
    pub since: u64,
    pub afk: bool,
}

impl Default for IdentifyPresence {
    /// Per the docs: when not re-identifying with a last known presence, "the
    /// status should be `unknown` and activities an empty array".
    fn default() -> Self {
        IdentifyPresence {
            activities: Vec::new(),
            status: "unknown".to_owned(),
            since: 0,
            afk: false,
        }
    }
}

/// Opcode 2 Identify data.
///
// ASSUMPTION: `capabilities` and `client_state` are optional and omitted. With
// no capabilities the gateway is expected to send the plain (non-deduped,
// non-versioned) READY shape that `model::Ready` parses. `model::Guild` also
// accepts the CLIENT_STATE_V2 shape in case that expectation is wrong.
#[derive(Clone, Debug, Serialize)]
pub struct Identify {
    #[serde(serialize_with = "serialize_exposed")]
    pub token: Secret<String>,
    pub properties: ClientProperties,
    pub compress: bool,
    pub presence: IdentifyPresence,
}

/// Opcode 6 Resume data.
#[derive(Clone, Debug, Serialize)]
pub struct Resume {
    #[serde(serialize_with = "serialize_exposed")]
    pub token: Secret<String>,
    pub session_id: String,
    pub seq: u64,
}

/// A payload the client sends. `Debug` is safe to log (tokens are redacted);
/// the output of [`GatewaySend::to_json`] is **not**.
#[derive(Clone, Debug)]
pub enum GatewaySend {
    /// Opcode 1 with the last received sequence number (`null` if none).
    Heartbeat(Option<u64>),
    /// Subscribes only to the visible member-list page. No background enumeration.
    GuildSubscription {
        guild_id: crate::Snowflake,
        channel: Option<(crate::Snowflake, usize)>,
    },
    Identify(Box<Identify>),
    Resume(Resume),
}

impl GatewaySend {
    pub fn opcode(&self) -> Opcode {
        match self {
            GatewaySend::Heartbeat(_) => Opcode::Heartbeat,
            GatewaySend::GuildSubscription { .. } => Opcode::GuildSubscriptionsBulk,
            GatewaySend::Identify(_) => Opcode::Identify,
            GatewaySend::Resume(_) => Opcode::Resume,
        }
    }

    /// Encodes the payload as a text frame.
    ///
    /// For Identify/Resume the result contains the token: write it to the
    /// socket and nowhere else.
    pub fn to_json(&self) -> String {
        let d = match self {
            GatewaySend::Heartbeat(seq) => json!(seq),
            GatewaySend::GuildSubscription { guild_id, channel } => {
                let mut channels = serde_json::Map::new();
                if let Some((id, offset)) = channel {
                    channels.insert(id.to_string(), json!([[offset, offset + 99]]));
                }
                // ASSUMPTION: opcode 37's lazy-member subscription shape is
                // observed in permissively licensed clients; verify live.
                json!({"subscriptions": {guild_id.to_string(): {"typing":true,"activities":true,"threads":false,"member_updates":false,"members":[],"channels":channels,"thread_member_lists":[]}}})
            }
            GatewaySend::Identify(identify) => json!(identify),
            GatewaySend::Resume(resume) => json!(resume),
        };
        json!({ "op": self.opcode().as_u8(), "d": d }).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE_TOKEN: &str = "fake-token-for-tests-only";

    fn identify() -> GatewaySend {
        GatewaySend::Identify(Box::new(Identify {
            token: Secret::new(FAKE_TOKEN.to_owned()),
            properties: ClientProperties {
                os: "linux".into(),
                browser: "dc-client".into(),
                device: "dc-client".into(),
            },
            compress: false,
            presence: IdentifyPresence::default(),
        }))
    }

    #[test]
    fn opcode_round_trip() {
        for value in 0..=43u8 {
            let op = Opcode::from_u8(value).expect("documented opcode");
            assert_eq!(op.as_u8(), value);
        }
        assert_eq!(Opcode::from_u8(44), None);
        assert_eq!(Opcode::from_u8(255), None);
    }

    #[test]
    fn heartbeat_encoding() {
        let none: Value = serde_json::from_str(&GatewaySend::Heartbeat(None).to_json()).unwrap();
        assert_eq!(none, json!({"op": 1, "d": null}));
        let some: Value =
            serde_json::from_str(&GatewaySend::Heartbeat(Some(251)).to_json()).unwrap();
        assert_eq!(some, json!({"op": 1, "d": 251}));
    }

    #[test]
    fn identify_encoding_matches_docs_shape() {
        let value: Value = serde_json::from_str(&identify().to_json()).unwrap();
        assert_eq!(
            value,
            json!({
                "op": 2,
                "d": {
                    "token": FAKE_TOKEN,
                    "properties": {"os": "linux", "browser": "dc-client", "device": "dc-client"},
                    "compress": false,
                    "presence": {"activities": [], "status": "unknown", "since": 0, "afk": false}
                }
            })
        );
    }

    #[test]
    fn resume_encoding() {
        let resume = GatewaySend::Resume(Resume {
            token: Secret::new(FAKE_TOKEN.to_owned()),
            session_id: "fake-session".into(),
            seq: 1337,
        });
        let value: Value = serde_json::from_str(&resume.to_json()).unwrap();
        assert_eq!(
            value,
            json!({"op": 6, "d": {"token": FAKE_TOKEN, "session_id": "fake-session", "seq": 1337}})
        );
    }

    #[test]
    fn debug_of_send_payloads_never_contains_token() {
        let resume = GatewaySend::Resume(Resume {
            token: Secret::new(FAKE_TOKEN.to_owned()),
            session_id: "fake-session".into(),
            seq: 1,
        });
        for payload in [identify(), resume] {
            let rendered = format!("{payload:?} {payload:#?}");
            assert!(!rendered.contains(FAKE_TOKEN), "leaked: {rendered}");
            assert!(rendered.contains("[REDACTED]"));
        }
    }

    #[test]
    fn gateway_url_adds_version_and_encoding() {
        assert_eq!(
            gateway_url("wss://gateway.example.invalid"),
            "wss://gateway.example.invalid/?v=9&encoding=json"
        );
        assert_eq!(
            gateway_url("wss://gateway.example.invalid/"),
            "wss://gateway.example.invalid/?v=9&encoding=json"
        );
    }

    #[test]
    fn unknown_opcode_is_not_an_error() {
        let parsed = GatewayReceive::parse(r#"{"op":99,"d":{"x":1}}"#).unwrap();
        assert_eq!(parsed, GatewayReceive::Unknown { op: 99 });
    }

    #[test]
    fn malformed_frames_are_errors() {
        assert!(GatewayReceive::parse("not json").is_err());
        assert!(GatewayReceive::parse(r#"{"d":{}}"#).is_err());
        assert!(GatewayReceive::parse(r#"{"op":10,"d":{}}"#).is_err());
    }
}
