//! Sans-IO gateway session state machine.
//!
//! The driver feeds it what happened (`on_connected`, `on_text`, `on_closed`,
//! `on_tick`) together with the current time, and it answers with [`Action`]s
//! to perform. It owns every protocol decision: when to heartbeat, whether to
//! Identify or Resume, when a connection is a zombie, and how long to back
//! off before reconnecting.

use std::time::Duration;

use dc_protocol::Secret;
use dc_protocol::close::{CloseAction, close_action, code};
use dc_protocol::gateway::{
    ClientProperties, DispatchEvent, GatewayReceive, GatewaySend, Identify, IdentifyPresence,
    Resume,
};

/// Close code used when *we* drop a connection but want to keep the session
/// resumable. The docs: closing with anything other than 1000/1001 keeps the
/// session alive for resuming.
const CLOSE_KEEP_SESSION: u16 = code::UNKNOWN_ERROR;

/// Tunables for a [`Session`].
#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub properties: ClientProperties,
    /// First reconnect delay; doubles on every consecutive failure.
    pub backoff_base: Duration,
    /// Upper bound for the reconnect delay.
    pub backoff_max: Duration,
    /// How long to wait for Hello after the socket opens.
    pub hello_timeout: Duration,
}

impl SessionConfig {
    pub fn new(properties: ClientProperties) -> Self {
        SessionConfig {
            properties,
            backoff_base: Duration::from_secs(1),
            backoff_max: Duration::from_secs(60),
            hello_timeout: Duration::from_secs(30),
        }
    }
}

/// Why the session gave up for good.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FatalReason {
    /// Close code 4004: the token was rejected. Do not retry with it.
    #[error("authentication failed (close code 4004): the token was rejected")]
    AuthenticationFailed,
    /// Another close code that reconnecting cannot fix.
    #[error("gateway closed the connection with non-recoverable close code {0}")]
    Closed(u16),
}

/// Something the driver must do.
#[derive(Debug)]
pub enum Action {
    /// Write this payload to the socket.
    Send(GatewaySend),
    /// Hand this event to the application.
    Event(DispatchEvent),
    /// End the current connection and reconnect after `reconnect_after`.
    /// `close_code` is `None` when the socket is already closed.
    Disconnect {
        close_code: Option<u16>,
        reconnect_after: Duration,
    },
    /// End the current connection and stop.
    Fatal(FatalReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Disconnected,
    AwaitingHello,
    /// Identify/Resume sent, waiting for READY/RESUMED.
    Handshaking,
    Active,
}

/// Source of jitter in `[0, 1)`. Injected so tests are deterministic.
pub type Jitter = Box<dyn FnMut() -> f64 + Send + Sync>;

pub struct Session {
    token: Secret<String>,
    config: SessionConfig,
    jitter: Jitter,
    phase: Phase,

    // Resume state; survives reconnects.
    seq: Option<u64>,
    session_id: Option<String>,
    resume_url: Option<String>,

    // Per-connection state.
    heartbeat_interval: Option<Duration>,
    next_heartbeat: Option<Duration>,
    awaiting_ack: bool,
    last_heartbeat_sent: Option<Duration>,
    latency: Option<Duration>,
    hello_deadline: Option<Duration>,

    /// Consecutive reconnects since the last READY/RESUMED.
    attempt: u32,
}

impl Session {
    pub fn new(token: Secret<String>, config: SessionConfig, jitter: Jitter) -> Self {
        Session {
            token,
            config,
            jitter,
            phase: Phase::Disconnected,
            seq: None,
            session_id: None,
            resume_url: None,
            heartbeat_interval: None,
            next_heartbeat: None,
            awaiting_ack: false,
            last_heartbeat_sent: None,
            latency: None,
            hello_deadline: None,
            attempt: 0,
        }
    }

    /// The URL to connect to for a resume, if there is a session to resume.
    /// `None` means: connect to the base gateway URL and Identify.
    pub fn resume_url(&self) -> Option<&str> {
        if self.session_id.is_some() && self.seq.is_some() {
            self.resume_url.as_deref()
        } else {
            None
        }
    }

    /// Last sequence number received.
    pub fn seq(&self) -> Option<u64> {
        self.seq
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Round-trip time of the last acknowledged heartbeat.
    pub fn latency(&self) -> Option<Duration> {
        self.latency
    }

    /// True between READY/RESUMED and the next disconnect.
    pub fn is_active(&self) -> bool {
        self.phase == Phase::Active
    }

    /// When `on_tick` next needs to be called, if at all.
    pub fn next_deadline(&self) -> Option<Duration> {
        match self.phase {
            Phase::Disconnected => None,
            Phase::AwaitingHello => self.hello_deadline,
            Phase::Handshaking | Phase::Active => self.next_heartbeat,
        }
    }

    /// The socket opened.
    pub fn on_connected(&mut self, now: Duration) {
        self.reset_connection_state();
        self.phase = Phase::AwaitingHello;
        self.hello_deadline = Some(now + self.config.hello_timeout);
    }

    /// Opening the socket failed. Returns how long to wait before retrying.
    pub fn on_connect_failed(&mut self) -> Duration {
        // Docs ("Handling a Disconnect"): if the reconnect fails, open a new
        // connection with the cached base URL and re-identify.
        if self.resume_url().is_some() {
            self.clear_session();
        }
        self.phase = Phase::Disconnected;
        self.next_backoff()
    }

    /// A text frame arrived.
    pub fn on_text(&mut self, text: &str, now: Duration) -> Vec<Action> {
        if self.phase == Phase::Disconnected {
            return Vec::new();
        }
        let payload = match GatewayReceive::parse(text) {
            Ok(payload) => payload,
            Err(err) => {
                // Never log the frame itself; payloads can contain user data.
                tracing::warn!(error = %err, "ignoring undecodable gateway frame");
                return Vec::new();
            }
        };
        match payload {
            GatewayReceive::Hello(hello) => self.on_hello(hello.heartbeat_interval, now),
            GatewayReceive::HeartbeatAck => {
                self.awaiting_ack = false;
                if let Some(sent) = self.last_heartbeat_sent {
                    self.latency = Some(now.saturating_sub(sent));
                }
                Vec::new()
            }
            GatewayReceive::Heartbeat => {
                // Heartbeat request: answer immediately, keep the schedule.
                if self.heartbeat_interval.is_none() {
                    return Vec::new();
                }
                // Only scheduled heartbeats arm the zombie check, so a request
                // that lands just before a scheduled beat cannot be mistaken
                // for a missed ACK.
                vec![Action::Send(GatewaySend::Heartbeat(self.seq))]
            }
            GatewayReceive::Reconnect => {
                tracing::debug!("gateway requested reconnect");
                let delay = self.reconnect_delay(true);
                vec![self.disconnect(Some(CLOSE_KEEP_SESSION), delay)]
            }
            GatewayReceive::InvalidSession { resumable } => self.on_invalid_session(resumable),
            GatewayReceive::Dispatch { seq, name, data } => self.on_dispatch(seq, &name, data),
            GatewayReceive::Unknown { op } => {
                tracing::trace!(op, "ignoring unhandled gateway opcode");
                Vec::new()
            }
        }
    }

    /// The connection ended (close frame, EOF or transport error).
    pub fn on_closed(&mut self, close_code: Option<u16>) -> Action {
        if self.phase == Phase::Disconnected {
            // Already handled (e.g. we initiated the disconnect).
            return Action::Disconnect {
                close_code: None,
                reconnect_after: Duration::ZERO,
            };
        }
        match close_action(close_code) {
            CloseAction::Resume => {
                let delay = self.next_backoff();
                self.disconnect(None, delay)
            }
            CloseAction::Reidentify => {
                self.clear_session();
                let delay = self.next_backoff();
                self.disconnect(None, delay)
            }
            CloseAction::Fatal => {
                self.clear_session();
                self.phase = Phase::Disconnected;
                self.reset_connection_state();
                Action::Fatal(match close_code {
                    Some(code::AUTHENTICATION_FAILED) => FatalReason::AuthenticationFailed,
                    Some(other) => FatalReason::Closed(other),
                    None => FatalReason::Closed(0),
                })
            }
        }
    }

    /// A deadline from [`Session::next_deadline`] may have passed.
    pub fn on_tick(&mut self, now: Duration) -> Vec<Action> {
        match self.phase {
            Phase::Disconnected => Vec::new(),
            Phase::AwaitingHello => {
                if self.hello_deadline.is_some_and(|deadline| now >= deadline) {
                    tracing::warn!("no Hello from gateway; reconnecting");
                    let delay = self.next_backoff();
                    vec![self.disconnect(Some(CLOSE_KEEP_SESSION), delay)]
                } else {
                    Vec::new()
                }
            }
            Phase::Handshaking | Phase::Active => {
                if !self.next_heartbeat.is_some_and(|due| now >= due) {
                    return Vec::new();
                }
                if self.awaiting_ack {
                    // Zombied connection: the previous heartbeat was never
                    // acknowledged. Close with a non-1000 code and resume.
                    tracing::warn!("heartbeat not acknowledged; reconnecting");
                    let delay = self.next_backoff();
                    return vec![self.disconnect(Some(CLOSE_KEEP_SESSION), delay)];
                }
                let interval = self.heartbeat_interval.unwrap_or(Duration::MAX);
                self.next_heartbeat = Some(now.saturating_add(interval));
                vec![self.heartbeat(now)]
            }
        }
    }

    fn on_hello(&mut self, interval_ms: u64, now: Duration) -> Vec<Action> {
        if self.phase != Phase::AwaitingHello {
            tracing::debug!("ignoring unexpected Hello");
            return Vec::new();
        }
        // Guard against a zero interval turning into a busy loop.
        let interval = Duration::from_millis(interval_ms.max(1));
        self.heartbeat_interval = Some(interval);
        self.hello_deadline = None;
        // First heartbeat after interval * jitter, then every interval.
        let jitter = (self.jitter)().clamp(0.0, 1.0);
        self.next_heartbeat = Some(now + interval.mul_f64(jitter));
        self.phase = Phase::Handshaking;

        let handshake = match (&self.session_id, self.seq) {
            (Some(session_id), Some(seq)) => GatewaySend::Resume(Resume {
                token: self.token.clone(),
                session_id: session_id.clone(),
                seq,
            }),
            _ => GatewaySend::Identify(Box::new(Identify {
                token: self.token.clone(),
                properties: self.config.properties.clone(),
                compress: false,
                presence: IdentifyPresence::default(),
            })),
        };
        vec![Action::Send(handshake)]
    }

    fn on_invalid_session(&mut self, resumable: bool) -> Vec<Action> {
        tracing::debug!(resumable, "gateway invalidated the session");
        if resumable && self.session_id.is_some() {
            let delay = self.next_backoff();
            return vec![self.disconnect(Some(CLOSE_KEEP_SESSION), delay)];
        }
        self.clear_session();
        // ASSUMPTION: the user docs allow re-identifying on the same socket;
        // we take the simpler, always-valid path of a fresh connection, and
        // wait a random 1-5 s first as the official bot docs recommend (this
        // also covers the identify concurrency limit).
        let jitter = (self.jitter)().clamp(0.0, 1.0);
        let wait = Duration::from_secs(1) + Duration::from_secs(4).mul_f64(jitter);
        let delay = wait.max(self.next_backoff());
        vec![self.disconnect(Some(code::NORMAL), delay)]
    }

    fn on_dispatch(
        &mut self,
        seq: Option<u64>,
        name: &str,
        data: serde_json::Value,
    ) -> Vec<Action> {
        if let Some(seq) = seq {
            self.seq = Some(seq);
        }
        let event = match DispatchEvent::decode(name, data) {
            Ok(event) => event,
            Err(err) => {
                // A known event with a surprising shape must not kill the
                // connection. The sequence number was still recorded above.
                tracing::warn!(event = name, error = %err, "dropping undecodable event");
                return Vec::new();
            }
        };
        match &event {
            DispatchEvent::Ready(ready) => {
                self.session_id = Some(ready.session_id.clone());
                self.resume_url = ready.resume_gateway_url.clone();
                self.phase = Phase::Active;
                self.attempt = 0;
            }
            DispatchEvent::Resumed => {
                self.phase = Phase::Active;
                self.attempt = 0;
            }
            _ => {}
        }
        vec![Action::Event(event)]
    }

    fn heartbeat(&mut self, now: Duration) -> Action {
        self.awaiting_ack = true;
        self.last_heartbeat_sent = Some(now);
        Action::Send(GatewaySend::Heartbeat(self.seq))
    }

    fn disconnect(&mut self, close_code: Option<u16>, reconnect_after: Duration) -> Action {
        self.phase = Phase::Disconnected;
        self.reset_connection_state();
        Action::Disconnect {
            close_code,
            reconnect_after,
        }
    }

    fn reset_connection_state(&mut self) {
        self.heartbeat_interval = None;
        self.next_heartbeat = None;
        self.awaiting_ack = false;
        self.last_heartbeat_sent = None;
        self.hello_deadline = None;
    }

    fn clear_session(&mut self) {
        self.session_id = None;
        self.resume_url = None;
        self.seq = None;
    }

    /// Exponential backoff with jitter: `base * 2^attempt`, capped at
    /// `backoff_max`, scaled into `[50%, 100%]`.
    fn next_backoff(&mut self) -> Duration {
        let exp = self.attempt.min(16);
        let uncapped = self.config.backoff_base.saturating_mul(1u32 << exp);
        let capped = uncapped.min(self.config.backoff_max);
        self.attempt = self.attempt.saturating_add(1);
        let jitter = (self.jitter)().clamp(0.0, 1.0);
        capped.mul_f64(0.5 + 0.5 * jitter)
    }

    /// Like `next_backoff`, but the first attempt may be immediate (used for
    /// opcode 7, where the docs say to reconnect "immediately").
    fn reconnect_delay(&mut self, immediate_ok: bool) -> Duration {
        if immediate_ok && self.attempt == 0 {
            self.attempt = 1;
            Duration::ZERO
        } else {
            self.next_backoff()
        }
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("phase", &self.phase)
            .field("seq", &self.seq)
            .field("has_session", &self.session_id.is_some())
            .field("attempt", &self.attempt)
            .finish_non_exhaustive()
    }
}
