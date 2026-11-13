//! Driver tests: `GatewayClient` against the mock transport and fake clock.
//!
//! Everything runs on a current-thread runtime; `settle()` lets the spawned
//! client task run until it is parked on the transport or the clock, so the
//! tests are deterministic and never sleep for real.

macro_rules! fixture {
    ($path:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/",
            $path
        ))
    };
}

mod common;

use std::time::Duration;

use common::*;
use dc_gateway::transport::mock::{MockConnector, MockServer, Sent};
use dc_gateway::{FatalReason, GatewayClient, GatewayError, GatewayEvent};
use dc_protocol::gateway::DispatchEvent;
use dc_protocol::time::{Clock, FakeClock};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

struct Harness {
    connector: MockConnector,
    clock: FakeClock,
    events: mpsc::Receiver<GatewayEvent>,
    task: JoinHandle<Result<(), GatewayError>>,
}

/// Starts a client with jitter fixed at 1.0 (first heartbeat after one full
/// interval, backoff at its nominal value).
fn start(connector: MockConnector) -> Harness {
    let clock = FakeClock::new();
    let (tx, events) = mpsc::channel(64);
    let client = GatewayClient::new(connector.clone(), clock.clone(), session(1.0), BASE_URL);
    let task = tokio::spawn(client.run(tx));
    Harness {
        connector,
        clock,
        events,
        task,
    }
}

async fn settle() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}

impl Harness {
    async fn advance(&self, by: Duration) {
        self.clock.advance(by);
        settle().await;
    }

    fn next_event(&mut self) -> Option<GatewayEvent> {
        self.events.try_recv().ok()
    }

    fn expect_dispatch(&mut self) -> DispatchEvent {
        match self.next_event() {
            Some(GatewayEvent::Dispatch(event)) => event,
            other => panic!("expected a dispatch, got {other:?}"),
        }
    }

    fn expect_reconnecting(&mut self) -> Duration {
        match self.next_event() {
            Some(GatewayEvent::Reconnecting { delay }) => delay,
            other => panic!("expected Reconnecting, got {other:?}"),
        }
    }
}

fn sent_op(server: &mut MockServer) -> u64 {
    match server.next_sent() {
        Some(Sent::Text(text)) => json(&text)["op"].as_u64().expect("op"),
        other => panic!("expected a text frame, got {other:?}"),
    }
}

/// Hello -> Identify -> READY on a fresh connection.
async fn handshake(harness: &mut Harness, server: &mut MockServer) {
    server.send_text(HELLO);
    settle().await;
    assert_eq!(sent_op(server), 2, "identify");
    server.send_text(READY);
    settle().await;
    assert!(matches!(harness.expect_dispatch(), DispatchEvent::Ready(_)));
}

#[tokio::test]
async fn identifies_then_streams_events() {
    let connector = MockConnector::new();
    let mut server = connector.expect_connection();
    let mut harness = start(connector);
    settle().await;
    assert_eq!(harness.connector.urls(), [BASE_WS]);

    // Nothing is sent before Hello.
    assert_eq!(server.next_sent(), None);
    handshake(&mut harness, &mut server).await;

    server.send_text(MESSAGE_CREATE);
    server.send_text(UNKNOWN_EVENT);
    settle().await;
    let DispatchEvent::MessageCreate(message) = harness.expect_dispatch() else {
        panic!("expected MESSAGE_CREATE");
    };
    assert_eq!(message.content, "hello from a fixture");
    assert!(matches!(
        harness.expect_dispatch(),
        DispatchEvent::Unknown { .. }
    ));
    assert_eq!(server.next_sent(), None);
    assert!(!harness.task.is_finished());
}

#[tokio::test]
async fn heartbeats_on_schedule_using_the_fake_clock() {
    let connector = MockConnector::new();
    let mut server = connector.expect_connection();
    let mut harness = start(connector);
    settle().await;
    handshake(&mut harness, &mut server).await;

    // Parked on the clock until the first beat.
    assert_eq!(harness.clock.next_wakeup_in(), Some(INTERVAL));
    harness.advance(INTERVAL - Duration::from_millis(1)).await;
    assert_eq!(server.next_sent(), None);

    harness.advance(Duration::from_millis(1)).await;
    assert_eq!(
        server.next_sent().map(|s| match s {
            Sent::Text(text) => json(&text),
            other => panic!("unexpected {other:?}"),
        }),
        Some(serde_json::json!({"op": 1, "d": 1}))
    );
    server.send_text(HEARTBEAT_ACK);
    settle().await;

    for _ in 0..3 {
        harness.advance(INTERVAL).await;
        assert_eq!(sent_op(&mut server), 1);
        server.send_text(HEARTBEAT_ACK);
        settle().await;
    }
    assert_eq!(server.next_sent(), None);
    assert_eq!(harness.clock.now(), INTERVAL * 4);
}

#[tokio::test]
async fn zombie_connection_is_closed_and_resumed_on_the_resume_url() {
    let connector = MockConnector::new();
    let mut first = connector.expect_connection();
    let mut second = connector.expect_connection();
    let mut harness = start(connector);
    settle().await;
    handshake(&mut harness, &mut first).await;

    harness.advance(INTERVAL).await;
    assert_eq!(sent_op(&mut first), 1);
    // No ACK. At the next beat the client gives up on this socket.
    harness.advance(INTERVAL).await;
    assert_eq!(first.next_sent(), Some(Sent::Close(4000)));
    assert_eq!(harness.expect_reconnecting(), Duration::from_secs(1));
    assert_eq!(harness.connector.urls().len(), 1, "still backing off");

    harness.advance(Duration::from_secs(1)).await;
    assert_eq!(harness.connector.urls(), [BASE_WS, RESUME_WS]);

    second.send_text(HELLO);
    settle().await;
    let Some(Sent::Text(resume)) = second.next_sent() else {
        panic!("expected resume frame");
    };
    let resume = json(&resume);
    assert_eq!(resume["op"], 6);
    assert_eq!(
        resume["d"]["session_id"],
        "fakesession0000000000000000000001"
    );
    assert_eq!(resume["d"]["seq"], 1);

    second.send_text(MESSAGE_CREATE);
    second.send_text(RESUMED);
    settle().await;
    assert!(matches!(
        harness.expect_dispatch(),
        DispatchEvent::MessageCreate(_)
    ));
    assert_eq!(harness.expect_dispatch(), DispatchEvent::Resumed);
}

#[tokio::test]
async fn reconnect_opcode_resumes_without_delay() {
    let connector = MockConnector::new();
    let mut first = connector.expect_connection();
    let mut second = connector.expect_connection();
    let mut harness = start(connector);
    settle().await;
    handshake(&mut harness, &mut first).await;

    first.send_text(RECONNECT);
    settle().await;
    assert_eq!(first.next_sent(), Some(Sent::Close(4000)));
    assert_eq!(harness.expect_reconnecting(), Duration::ZERO);
    // No clock advance needed.
    assert_eq!(harness.connector.urls(), [BASE_WS, RESUME_WS]);
    second.send_text(HELLO);
    settle().await;
    assert_eq!(sent_op(&mut second), 6);
}

#[tokio::test]
async fn invalid_session_reidentifies_on_the_base_url_after_waiting() {
    let connector = MockConnector::new();
    let mut first = connector.expect_connection();
    let mut second = connector.expect_connection();
    let mut harness = start(connector);
    settle().await;
    handshake(&mut harness, &mut first).await;

    first.send_text(INVALID_SESSION);
    settle().await;
    assert_eq!(first.next_sent(), Some(Sent::Close(1000)));
    // 1s + 4s * jitter(1.0)
    assert_eq!(harness.expect_reconnecting(), Duration::from_secs(5));
    harness.advance(Duration::from_secs(4)).await;
    assert_eq!(harness.connector.urls().len(), 1);
    harness.advance(Duration::from_secs(1)).await;
    assert_eq!(harness.connector.urls(), [BASE_WS, BASE_WS]);

    second.send_text(HELLO);
    settle().await;
    assert_eq!(sent_op(&mut second), 2);
}

#[tokio::test]
async fn connect_failures_back_off_exponentially() {
    let connector = MockConnector::new();
    connector.expect_failure();
    connector.expect_failure();
    connector.expect_failure();
    let mut server = connector.expect_connection();
    let mut harness = start(connector);
    settle().await;

    for (attempt, expected) in [1u64, 2, 4].into_iter().enumerate() {
        assert_eq!(harness.connector.urls().len(), attempt + 1);
        assert_eq!(harness.expect_reconnecting(), Duration::from_secs(expected));
        // One millisecond short: still waiting.
        harness
            .advance(Duration::from_secs(expected) - Duration::from_millis(1))
            .await;
        assert_eq!(harness.connector.urls().len(), attempt + 1);
        harness.advance(Duration::from_millis(1)).await;
    }
    assert_eq!(harness.connector.urls().len(), 4);
    handshake(&mut harness, &mut server).await;
}

#[tokio::test]
async fn abrupt_close_resumes_and_failed_resume_connect_reidentifies() {
    let connector = MockConnector::new();
    let mut first = connector.expect_connection();
    connector.expect_failure(); // the resume attempt cannot connect
    let mut third = connector.expect_connection();
    let mut harness = start(connector);
    settle().await;
    handshake(&mut harness, &mut first).await;

    drop(first); // EOF without a close frame
    settle().await;
    assert_eq!(harness.expect_reconnecting(), Duration::from_secs(1));
    harness.advance(Duration::from_secs(1)).await;
    assert_eq!(harness.connector.urls(), [BASE_WS, RESUME_WS]);

    assert_eq!(harness.expect_reconnecting(), Duration::from_secs(2));
    harness.advance(Duration::from_secs(2)).await;
    // Per the docs: failed reconnect -> base URL and a fresh Identify.
    assert_eq!(harness.connector.urls(), [BASE_WS, RESUME_WS, BASE_WS]);
    third.send_text(HELLO);
    settle().await;
    assert_eq!(sent_op(&mut third), 2);
}

#[tokio::test]
async fn authentication_failure_is_fatal_and_never_retried() {
    let connector = MockConnector::new();
    let mut server = connector.expect_connection();
    let harness = start(connector);
    settle().await;
    server.send_text(HELLO);
    settle().await;
    assert_eq!(sent_op(&mut server), 2);

    server.close(Some(4004));
    settle().await;
    let result = harness.task.await.expect("task must not panic");
    assert!(matches!(
        result,
        Err(GatewayError::Fatal(FatalReason::AuthenticationFailed))
    ));
    assert_eq!(harness.connector.urls().len(), 1, "no reconnect attempt");
    assert_eq!(harness.clock.pending_sleeps(), 0);
    // The error text is safe to print.
    let message = format!("{:?} {}", result, result.as_ref().unwrap_err());
    assert!(!message.contains(FAKE_TOKEN));
}

#[tokio::test]
async fn dropping_the_event_receiver_closes_the_session_cleanly() {
    let connector = MockConnector::new();
    let mut server = connector.expect_connection();
    let mut harness = start(connector);
    settle().await;
    handshake(&mut harness, &mut server).await;

    drop(harness.events);
    settle().await;
    assert!(matches!(harness.task.await, Ok(Ok(()))));
    assert_eq!(server.next_sent(), Some(Sent::Close(1000)));
}

#[tokio::test]
async fn member_commands_wait_for_ready_and_are_throttled_without_blocking_events() {
    use dc_gateway::GatewayCommand;
    use dc_protocol::Snowflake;
    let connector = MockConnector::new();
    let mut server = connector.expect_connection();
    let clock = FakeClock::new();
    let (events_tx, mut events) = mpsc::channel(64);
    let (commands, command_rx) = mpsc::channel(8);
    let client = GatewayClient::new(connector, clock.clone(), session(1.0), BASE_URL)
        .with_commands(command_rx);
    let task = tokio::spawn(client.run(events_tx));
    commands
        .send(GatewayCommand::Members {
            guild: Snowflake(1),
            channel: Snowflake(2),
            offset: 100,
        })
        .await
        .unwrap();
    settle().await;
    assert_eq!(server.next_sent(), None);
    server.send_text(HELLO);
    settle().await;
    assert_eq!(sent_op(&mut server), 2);
    assert_eq!(server.next_sent(), None);
    server.send_text(READY);
    settle().await;
    assert!(matches!(
        events.try_recv(),
        Ok(GatewayEvent::Dispatch(DispatchEvent::Ready(_)))
    ));
    assert_eq!(sent_op(&mut server), 37);
    assert_eq!(server.next_sent(), None);
    server.send_text(MESSAGE_CREATE);
    settle().await;
    assert!(matches!(
        events.try_recv(),
        Ok(GatewayEvent::Dispatch(DispatchEvent::MessageCreate(_)))
    ));
    clock.advance(Duration::from_millis(749));
    settle().await;
    assert_eq!(server.next_sent(), None);
    clock.advance(Duration::from_millis(1));
    settle().await;
    let Some(Sent::Text(packet)) = server.next_sent() else {
        panic!("missing subscription");
    };
    let packet = json(&packet);
    assert_eq!(
        packet["d"]["subscriptions"]["1"]["channels"]["2"],
        serde_json::json!([[100, 199]])
    );
    drop(events);
    settle().await;
    assert!(task.await.unwrap().is_ok());
}
