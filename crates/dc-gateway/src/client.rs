//! Async driver: connects, runs the [`Session`] state machine against a
//! [`Transport`], reconnects with backoff, and emits events.

use std::time::Duration;

use dc_protocol::gateway::{DispatchEvent, gateway_url};
use dc_protocol::time::Clock;
use tokio::sync::mpsc;

use crate::session::{Action, FatalReason, Session};
use crate::transport::{Connector, Incoming, Transport};

/// What the driver reports to the application.
#[derive(Debug)]
pub enum GatewayEvent {
    /// A dispatch from Discord (READY, MESSAGE_CREATE, ...).
    Dispatch(DispatchEvent),
    /// The connection dropped; a new attempt starts after `delay`.
    Reconnecting { delay: Duration },
}

/// Why [`GatewayClient::run`] stopped with an error.
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error(transparent)]
    Fatal(#[from] FatalReason),
}

enum Flow {
    Continue,
    Reconnect(Duration),
    Fatal(FatalReason),
    Shutdown,
}

/// A gateway connection manager.
pub struct GatewayClient<Cn, Ck> {
    connector: Cn,
    clock: Ck,
    session: Session,
    base_url: String,
}

impl<Cn: Connector, Ck: Clock> GatewayClient<Cn, Ck> {
    /// `base_url` is the `url` from `GET /gateway` (e.g. `wss://...`),
    /// without query parameters.
    pub fn new(connector: Cn, clock: Ck, session: Session, base_url: impl Into<String>) -> Self {
        GatewayClient {
            connector,
            clock,
            session,
            base_url: base_url.into(),
        }
    }

    /// Runs until the receiver of `events` is dropped (returns `Ok`) or the
    /// gateway reports a non-recoverable error.
    pub async fn run(mut self, events: mpsc::Sender<GatewayEvent>) -> Result<(), GatewayError> {
        loop {
            let url = gateway_url(self.session.resume_url().unwrap_or(&self.base_url));
            let connect = tokio::select! {
                biased;
                () = events.closed() => return Ok(()),
                result = self.connector.connect(&url) => result,
            };
            let mut conn = match connect {
                Ok(conn) => conn,
                Err(err) => {
                    let delay = self.session.on_connect_failed();
                    tracing::warn!(error = %err, ?delay, "gateway connect failed");
                    if !self.wait_before_reconnect(delay, &events).await {
                        return Ok(());
                    }
                    continue;
                }
            };
            tracing::debug!("gateway socket open");
            self.session.on_connected(self.clock.now());

            let flow = self.run_connection(&mut conn, &events).await;
            match flow {
                Flow::Continue => unreachable!("run_connection never yields Continue"),
                Flow::Reconnect(delay) => {
                    if !self.wait_before_reconnect(delay, &events).await {
                        return Ok(());
                    }
                }
                Flow::Fatal(reason) => return Err(reason.into()),
                Flow::Shutdown => return Ok(()),
            }
        }
    }

    /// Drives one connection until it ends.
    async fn run_connection(
        &mut self,
        conn: &mut Cn::Conn,
        events: &mpsc::Sender<GatewayEvent>,
    ) -> Flow {
        loop {
            let now = self.clock.now();
            let sleep_for = self
                .session
                .next_deadline()
                .map(|deadline| deadline.saturating_sub(now));

            let actions = tokio::select! {
                biased;
                () = events.closed() => {
                    // Application went away: end the session cleanly.
                    conn.close(dc_protocol::close::code::NORMAL).await;
                    return Flow::Shutdown;
                }
                incoming = conn.recv() => match incoming {
                    Ok(Incoming::Text(text)) => self.session.on_text(&text, self.clock.now()),
                    Ok(Incoming::Closed(code)) => {
                        tracing::debug!(?code, "gateway connection closed");
                        vec![self.session.on_closed(code)]
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "gateway transport error");
                        vec![self.session.on_closed(None)]
                    }
                },
                () = sleep_opt(&self.clock, sleep_for) => self.session.on_tick(self.clock.now()),
            };

            for action in actions {
                match self.apply(conn, events, action).await {
                    Flow::Continue => {}
                    flow => return flow,
                }
            }
        }
    }

    async fn apply(
        &mut self,
        conn: &mut Cn::Conn,
        events: &mpsc::Sender<GatewayEvent>,
        action: Action,
    ) -> Flow {
        match action {
            Action::Send(payload) => {
                // `payload` Debug is redacted; the JSON is not, so it goes
                // straight to the transport.
                tracing::trace!(op = ?payload.opcode(), "gateway send");
                if let Err(err) = conn.send(payload.to_json()).await {
                    tracing::warn!(error = %err, "gateway send failed");
                    return match self.session.on_closed(None) {
                        Action::Fatal(reason) => Flow::Fatal(reason),
                        Action::Disconnect {
                            reconnect_after, ..
                        } => Flow::Reconnect(reconnect_after),
                        _ => Flow::Reconnect(Duration::ZERO),
                    };
                }
                Flow::Continue
            }
            Action::Event(event) => {
                if events.send(GatewayEvent::Dispatch(event)).await.is_err() {
                    conn.close(dc_protocol::close::code::NORMAL).await;
                    return Flow::Shutdown;
                }
                Flow::Continue
            }
            Action::Disconnect {
                close_code,
                reconnect_after,
            } => {
                if let Some(code) = close_code {
                    conn.close(code).await;
                }
                Flow::Reconnect(reconnect_after)
            }
            Action::Fatal(reason) => Flow::Fatal(reason),
        }
    }

    /// Returns `false` if the application went away while waiting.
    async fn wait_before_reconnect(
        &self,
        delay: Duration,
        events: &mpsc::Sender<GatewayEvent>,
    ) -> bool {
        if events
            .send(GatewayEvent::Reconnecting { delay })
            .await
            .is_err()
        {
            return false;
        }
        tokio::select! {
            biased;
            () = events.closed() => false,
            () = self.clock.sleep(delay) => true,
        }
    }
}

async fn sleep_opt<C: Clock>(clock: &C, duration: Option<Duration>) {
    match duration {
        Some(duration) => clock.sleep(duration).await,
        None => std::future::pending().await,
    }
}
