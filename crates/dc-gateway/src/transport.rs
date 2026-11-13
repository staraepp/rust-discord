//! Transport abstraction for the gateway driver, plus an in-memory mock.

use std::future::Future;

/// A transport-level failure. The message must never contain payload data.
#[derive(Clone, Debug, thiserror::Error)]
#[error("gateway transport error: {0}")]
pub struct TransportError(pub String);

/// What arrived on a connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    /// A complete text frame (one JSON gateway payload).
    Text(String),
    /// The peer closed the connection, with its close code if it sent one.
    Closed(Option<u16>),
}

/// Opens gateway connections.
pub trait Connector: Send + Sync + 'static {
    type Conn: Transport;

    fn connect(&self, url: &str)
    -> impl Future<Output = Result<Self::Conn, TransportError>> + Send;
}

/// One open gateway connection.
pub trait Transport: Send + 'static {
    /// Sends one text frame. The frame may contain the token: implementations
    /// must not log it.
    fn send(&mut self, text: String) -> impl Future<Output = Result<(), TransportError>> + Send;

    /// Waits for the next frame. Must be cancel-safe: dropping the future
    /// before it completes must not lose a frame.
    fn recv(&mut self) -> impl Future<Output = Result<Incoming, TransportError>> + Send;

    /// Closes the connection with the given close code. Best effort.
    fn close(&mut self, code: u16) -> impl Future<Output = ()> + Send;
}

pub mod mock {
    //! In-memory transport for deterministic tests.
    //!
    //! ```ignore
    //! let connector = MockConnector::new();
    //! let mut server = connector.expect_connection(); // scripted "server" side
    //! server.send_text(r#"{"op":10,"d":{"heartbeat_interval":1000}}"#);
    //! // ... run the client ...
    //! assert!(matches!(server.next_sent(), Some(Sent::Text(_))));
    //! ```

    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex, MutexGuard};

    use tokio::sync::mpsc;

    use super::{Connector, Incoming, Transport, TransportError};

    /// Something the client did on a mock connection.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Sent {
        Text(String),
        Close(u16),
    }

    #[derive(Default)]
    struct Shared {
        pending: VecDeque<Option<MockTransport>>,
        urls: Vec<String>,
    }

    /// Hands out scripted connections in the order they were queued.
    #[derive(Clone, Default)]
    pub struct MockConnector {
        shared: Arc<Mutex<Shared>>,
    }

    impl MockConnector {
        pub fn new() -> Self {
            Self::default()
        }

        /// Queues a connection that will succeed; returns its server side.
        pub fn expect_connection(&self) -> MockServer {
            let (to_client, from_server) = mpsc::unbounded_channel();
            let (to_server, from_client) = mpsc::unbounded_channel();
            self.lock().pending.push_back(Some(MockTransport {
                incoming: from_server,
                outgoing: to_server,
            }));
            MockServer {
                to_client,
                from_client,
            }
        }

        /// Queues a connection attempt that will fail.
        pub fn expect_failure(&self) {
            self.lock().pending.push_back(None);
        }

        /// Every URL the client tried to connect to, in order.
        pub fn urls(&self) -> Vec<String> {
            self.lock().urls.clone()
        }

        fn lock(&self) -> MutexGuard<'_, Shared> {
            self.shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        }
    }

    impl Connector for MockConnector {
        type Conn = MockTransport;

        async fn connect(&self, url: &str) -> Result<MockTransport, TransportError> {
            let mut shared = self.lock();
            shared.urls.push(url.to_owned());
            match shared.pending.pop_front() {
                Some(Some(transport)) => Ok(transport),
                Some(None) => Err(TransportError("scripted connect failure".into())),
                None => Err(TransportError("no scripted connection left".into())),
            }
        }
    }

    /// Client side of a mock connection.
    pub struct MockTransport {
        incoming: mpsc::UnboundedReceiver<Incoming>,
        outgoing: mpsc::UnboundedSender<Sent>,
    }

    impl Transport for MockTransport {
        async fn send(&mut self, text: String) -> Result<(), TransportError> {
            self.outgoing
                .send(Sent::Text(text))
                .map_err(|_| TransportError("mock server dropped".into()))
        }

        async fn recv(&mut self) -> Result<Incoming, TransportError> {
            match self.incoming.recv().await {
                Some(incoming) => Ok(incoming),
                // Server handle dropped: abrupt EOF without a close frame.
                None => Ok(Incoming::Closed(None)),
            }
        }

        async fn close(&mut self, code: u16) {
            let _ = self.outgoing.send(Sent::Close(code));
        }
    }

    /// The scripted "server" side of a mock connection, held by the test.
    pub struct MockServer {
        to_client: mpsc::UnboundedSender<Incoming>,
        from_client: mpsc::UnboundedReceiver<Sent>,
    }

    impl MockServer {
        /// Delivers a text frame to the client.
        pub fn send_text(&self, text: impl Into<String>) {
            let _ = self.to_client.send(Incoming::Text(text.into()));
        }

        /// Closes the connection from the server side.
        pub fn close(&self, code: Option<u16>) {
            let _ = self.to_client.send(Incoming::Closed(code));
        }

        /// The next thing the client sent, if any (non-blocking).
        pub fn next_sent(&mut self) -> Option<Sent> {
            self.from_client.try_recv().ok()
        }

        /// Everything the client has sent so far (non-blocking).
        pub fn drain_sent(&mut self) -> Vec<Sent> {
            std::iter::from_fn(|| self.next_sent()).collect()
        }
    }
}
