//! Discord gateway client.
//!
//! Layers:
//! - [`session`]: a sans-IO state machine (heartbeat, identify, resume,
//!   reconnect/backoff decisions). Pure and fully deterministic.
//! - [`transport`]: the `Connector`/`Transport` traits the driver talks to,
//!   plus an in-memory mock for tests.
//! - [`client`]: the async driver that wires a session to a transport and a
//!   [`dc_protocol::time::Clock`] and produces an event stream.
//! - [`ws`] (feature `ws`): the real WebSocket transport and tokio clock.
//!
//! Tokens: frames produced by `GatewaySend::to_json` are written to the
//! transport and never logged.

pub mod client;
pub mod session;
pub mod transport;
#[cfg(feature = "ws")]
pub mod ws;

pub use client::{GatewayClient, GatewayError, GatewayEvent};
pub use session::{Action, FatalReason, Session, SessionConfig};
pub use transport::{Connector, Incoming, Transport, TransportError};

pub use client::GatewayCommand;
