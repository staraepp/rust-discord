//! Real transport: WebSocket over TLS (tokio-tungstenite + rustls), and the
//! tokio-backed clock.
//!
//! UNVERIFIED against the live gateway (cannot connect from the sandbox); see
//! `docs/LOCAL_VERIFY.md`.

use std::time::Duration;

use dc_protocol::time::Clock;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use crate::transport::{Connector, Incoming, Transport, TransportError};

/// Monotonic clock backed by tokio's timer.
#[derive(Clone, Debug)]
pub struct TokioClock {
    origin: tokio::time::Instant,
}

impl TokioClock {
    pub fn new() -> Self {
        TokioClock {
            origin: tokio::time::Instant::now(),
        }
    }
}

impl Default for TokioClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for TokioClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// Jitter source for production use: uniform in `[0, 1)`.
pub fn random_jitter() -> crate::session::Jitter {
    Box::new(fastrand::f64)
}

/// Connects with `wss://` using rustls and the bundled webpki roots.
#[derive(Clone, Copy, Debug, Default)]
pub struct WsConnector;

impl Connector for WsConnector {
    type Conn = WsTransport;

    async fn connect(&self, url: &str) -> Result<WsTransport, TransportError> {
        if !url.starts_with("wss://") {
            // Never send a token over an unencrypted socket.
            return Err(TransportError("refusing non-wss gateway URL".into()));
        }
        let (stream, _response) = connect_async(url)
            .await
            .map_err(|err| TransportError(err.to_string()))?;
        Ok(WsTransport { stream })
    }
}

pub struct WsTransport {
    stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

impl Transport for WsTransport {
    async fn send(&mut self, text: String) -> Result<(), TransportError> {
        self.stream
            .send(Message::text(text))
            .await
            .map_err(|err| TransportError(err.to_string()))
    }

    async fn recv(&mut self) -> Result<Incoming, TransportError> {
        loop {
            // `StreamExt::next` on a WebSocketStream is cancel-safe.
            match self.stream.next().await {
                Some(Ok(Message::Text(text))) => return Ok(Incoming::Text(text.to_string())),
                Some(Ok(Message::Close(frame))) => {
                    return Ok(Incoming::Closed(frame.map(|f| u16::from(f.code))));
                }
                // No transport compression is requested, so binary frames are
                // not expected; pings/pongs are answered by tungstenite.
                Some(Ok(_)) => continue,
                Some(Err(err)) => return Err(TransportError(err.to_string())),
                None => return Ok(Incoming::Closed(None)),
            }
        }
    }

    async fn close(&mut self, code: u16) {
        let frame = CloseFrame {
            code: code.into(),
            reason: "".into(),
        };
        let _ = self.stream.close(Some(frame)).await;
    }
}
