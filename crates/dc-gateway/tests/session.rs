//! State-machine tests for `Session` (pure, no IO, explicit time).

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
use dc_gateway::session::{Action, FatalReason, Session};
use dc_protocol::gateway::DispatchEvent;
use serde_json::json;

const T0: Duration = Duration::ZERO;

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

/// Asserts a single `Send` action and returns its JSON.
fn sent(actions: Vec<Action>) -> serde_json::Value {
    let [Action::Send(payload)] = &actions[..] else {
        panic!("expected exactly one Send, got {actions:?}");
    };
    json(&payload.to_json())
}

fn disconnect(actions: Vec<Action>) -> (Option<u16>, Duration) {
    let [
        Action::Disconnect {
            close_code,
            reconnect_after,
        },
    ] = &actions[..]
    else {
        panic!("expected exactly one Disconnect, got {actions:?}");
    };
    (*close_code, *reconnect_after)
}

/// Connects and completes the Identify -> READY handshake at time `now`.
fn establish(session: &mut Session, now: Duration) {
    session.on_connected(now);
    let identify = sent(session.on_text(HELLO, now));
    assert_eq!(identify["op"], 2);
    let actions = session.on_text(READY, now);
    assert!(matches!(
        &actions[..],
        [Action::Event(DispatchEvent::Ready(_))]
    ));
    assert!(session.is_active());
}

#[test]
fn hello_triggers_identify_with_token_and_properties() {
    let mut session = session(0.5);
    assert_eq!(session.resume_url(), None);
    session.on_connected(T0);
    assert_eq!(session.next_deadline(), Some(secs(30)), "hello timeout");

    let identify = sent(session.on_text(HELLO, T0));
    assert_eq!(
        identify,
        json!({
            "op": 2,
            "d": {
                "token": FAKE_TOKEN,
                "properties": {"os": "linux", "browser": "dc-test", "device": "dc-test"},
                "compress": false,
                "presence": {"activities": [], "status": "unknown", "since": 0, "afk": false}
            }
        })
    );
    // First heartbeat is due after interval * jitter.
    assert_eq!(session.next_deadline(), Some(INTERVAL.mul_f64(0.5)));
    assert!(!session.is_active());
}

#[test]
fn ready_stores_resume_state() {
    let mut session = session(0.0);
    establish(&mut session, T0);
    assert_eq!(
        session.session_id(),
        Some("fakesession0000000000000000000001")
    );
    assert_eq!(session.seq(), Some(1));
    assert_eq!(
        session.resume_url(),
        Some("wss://gateway-resume.example.invalid")
    );
}

#[test]
fn heartbeats_follow_the_interval_and_carry_the_sequence() {
    let mut session = session(0.5);
    session.on_connected(T0);
    session.on_text(HELLO, T0);

    let first_due = INTERVAL.mul_f64(0.5);
    // Not due yet: nothing happens.
    assert!(
        session
            .on_tick(first_due - Duration::from_millis(1))
            .is_empty()
    );

    // No dispatch received yet -> d is null.
    assert_eq!(
        sent(session.on_tick(first_due)),
        json!({"op": 1, "d": null})
    );
    assert_eq!(session.next_deadline(), Some(first_due + INTERVAL));

    let ack_at = first_due + Duration::from_millis(80);
    assert!(session.on_text(HEARTBEAT_ACK, ack_at).is_empty());
    assert_eq!(session.latency(), Some(Duration::from_millis(80)));

    session.on_text(READY, ack_at); // s = 1
    session.on_text(MESSAGE_CREATE, ack_at); // s = 2

    let second_due = first_due + INTERVAL;
    assert_eq!(sent(session.on_tick(second_due)), json!({"op": 1, "d": 2}));
    assert_eq!(session.next_deadline(), Some(second_due + INTERVAL));
}

#[test]
fn heartbeat_request_is_answered_immediately_without_shifting_the_schedule() {
    let mut session = session(1.0);
    establish(&mut session, T0);
    let deadline = session.next_deadline();

    let reply = sent(session.on_text(HEARTBEAT_REQUEST, secs(5)));
    assert_eq!(reply, json!({"op": 1, "d": 1}));
    assert_eq!(session.next_deadline(), deadline);

    // The on-demand beat does not arm the zombie check.
    assert_eq!(sent(session.on_tick(INTERVAL))["op"], 1);
}

#[test]
fn missed_ack_is_treated_as_a_zombie_connection_and_resumed() {
    let mut session = session(1.0);
    establish(&mut session, T0);

    assert_eq!(sent(session.on_tick(INTERVAL))["op"], 1);
    // No ACK arrives before the next beat is due.
    let (close_code, delay) = disconnect(session.on_tick(INTERVAL * 2));
    // Must not be 1000/1001, or the session would be invalidated.
    assert_eq!(close_code, Some(4000));
    assert_eq!(delay, secs(1));
    assert_eq!(session.next_deadline(), None);

    // Session survives: next connection resumes.
    assert_eq!(
        session.resume_url(),
        Some("wss://gateway-resume.example.invalid")
    );
    let now = INTERVAL * 2 + delay;
    session.on_connected(now);
    let resume = sent(session.on_text(HELLO, now));
    assert_eq!(
        resume,
        json!({
            "op": 6,
            "d": {
                "token": FAKE_TOKEN,
                "session_id": "fakesession0000000000000000000001",
                "seq": 1
            }
        })
    );
}

#[test]
fn acked_heartbeats_never_trip_the_zombie_check() {
    let mut session = session(1.0);
    establish(&mut session, T0);
    for beat in 1..=5u32 {
        let now = INTERVAL * beat;
        assert_eq!(sent(session.on_tick(now))["op"], 1, "beat {beat}");
        session.on_text(HEARTBEAT_ACK, now + Duration::from_millis(30));
    }
}

#[test]
fn resume_replays_events_and_resumed_marks_the_session_active() {
    let mut session = session(1.0);
    establish(&mut session, T0);
    session.on_text(MESSAGE_CREATE, T0); // seq 2
    let _ = session.on_closed(None);
    assert!(!session.is_active());

    session.on_connected(secs(1));
    let resume = sent(session.on_text(HELLO, secs(1)));
    assert_eq!(resume["op"], 6);
    assert_eq!(resume["d"]["seq"], 2);

    // Replayed event, then RESUMED (s = 12 in the fixture).
    let replay = session.on_text(UNKNOWN_EVENT, secs(1));
    assert!(matches!(
        &replay[..],
        [Action::Event(DispatchEvent::Unknown { .. })]
    ));
    assert!(!session.is_active());
    let resumed = session.on_text(RESUMED, secs(1));
    assert!(matches!(
        &resumed[..],
        [Action::Event(DispatchEvent::Resumed)]
    ));
    assert!(session.is_active());
    assert_eq!(session.seq(), Some(12));
}

#[test]
fn opcode_7_reconnects_immediately_and_resumes() {
    let mut session = session(1.0);
    establish(&mut session, T0);

    let (close_code, delay) = disconnect(session.on_text(RECONNECT, secs(10)));
    assert_eq!(close_code, Some(4000));
    assert_eq!(delay, Duration::ZERO);
    assert!(session.resume_url().is_some());

    session.on_connected(secs(10));
    assert_eq!(sent(session.on_text(HELLO, secs(10)))["op"], 6);

    // A second Reconnect before the resume completes backs off.
    let (_, delay) = disconnect(session.on_text(RECONNECT, secs(10)));
    assert_eq!(delay, secs(2));
}

#[test]
fn invalid_session_not_resumable_drops_the_session_and_reidentifies() {
    let mut session = session(0.5);
    establish(&mut session, T0);

    let (close_code, delay) = disconnect(session.on_text(INVALID_SESSION, secs(1)));
    assert_eq!(close_code, Some(1000));
    // 1s + 4s * jitter(0.5)
    assert_eq!(delay, secs(3));
    assert_eq!(session.resume_url(), None);
    assert_eq!(session.session_id(), None);
    assert_eq!(session.seq(), None);

    session.on_connected(secs(4));
    assert_eq!(sent(session.on_text(HELLO, secs(4)))["op"], 2);
}

#[test]
fn invalid_session_resumable_keeps_the_session() {
    let mut session = session(1.0);
    establish(&mut session, T0);

    let (close_code, _) = disconnect(session.on_text(INVALID_SESSION_RESUMABLE, secs(1)));
    assert_eq!(close_code, Some(4000));
    assert!(session.resume_url().is_some());

    session.on_connected(secs(2));
    assert_eq!(sent(session.on_text(HELLO, secs(2)))["op"], 6);
}

#[test]
fn invalid_session_after_failed_resume_falls_back_to_identify() {
    let mut session = session(0.0);
    establish(&mut session, T0);
    let _ = session.on_closed(None);

    session.on_connected(secs(1));
    assert_eq!(sent(session.on_text(HELLO, secs(1)))["op"], 6);
    let (_, delay) = disconnect(session.on_text(INVALID_SESSION, secs(1)));
    assert!(delay >= secs(1), "must wait before re-identifying");

    session.on_connected(secs(5));
    assert_eq!(sent(session.on_text(HELLO, secs(5)))["op"], 2);
}

#[test]
fn close_codes_decide_between_resume_reidentify_and_fatal() {
    // Resumable: no code, 4000.
    for code in [None, Some(4000), Some(4008)] {
        let mut session = common::session(1.0);
        establish(&mut session, T0);
        let action = session.on_closed(code);
        assert!(
            matches!(
                action,
                Action::Disconnect {
                    close_code: None,
                    ..
                }
            ),
            "{code:?}"
        );
        assert!(session.resume_url().is_some(), "{code:?} should resume");
    }
    // Session lost: 4007 invalid seq, 4009 timed out, 1000.
    for code in [4007, 4009, 4003, 1000] {
        let mut session = common::session(1.0);
        establish(&mut session, T0);
        let action = session.on_closed(Some(code));
        assert!(matches!(action, Action::Disconnect { .. }), "{code}");
        assert_eq!(session.resume_url(), None, "{code} should re-identify");
    }
    // Fatal.
    let mut session = common::session(1.0);
    establish(&mut session, T0);
    assert!(matches!(
        session.on_closed(Some(4004)),
        Action::Fatal(FatalReason::AuthenticationFailed)
    ));
    let mut session = common::session(1.0);
    establish(&mut session, T0);
    assert!(matches!(
        session.on_closed(Some(4012)),
        Action::Fatal(FatalReason::Closed(4012))
    ));
}

#[test]
fn backoff_doubles_caps_and_resets_after_ready() {
    let mut session = session(1.0);
    let delays: Vec<u64> = (0..9)
        .map(|_| session.on_connect_failed().as_secs())
        .collect();
    assert_eq!(delays, [1, 2, 4, 8, 16, 32, 60, 60, 60]);

    // A successful handshake resets the counter.
    establish(&mut session, T0);
    let Action::Disconnect {
        reconnect_after, ..
    } = session.on_closed(None)
    else {
        panic!("expected disconnect");
    };
    assert_eq!(reconnect_after, secs(1));
}

#[test]
fn backoff_jitter_scales_between_half_and_full() {
    let mut low = session(0.0);
    assert_eq!(low.on_connect_failed(), Duration::from_millis(500));
    let mut mid = session(0.5);
    assert_eq!(mid.on_connect_failed(), Duration::from_millis(750));
    // Out-of-range jitter sources are clamped.
    let mut wild = session(7.0);
    assert_eq!(wild.on_connect_failed(), secs(1));
}

#[test]
fn failed_resume_connect_falls_back_to_fresh_identify() {
    let mut session = session(1.0);
    establish(&mut session, T0);
    let _ = session.on_closed(None);
    assert!(session.resume_url().is_some());

    let _ = session.on_connect_failed();
    assert_eq!(session.resume_url(), None);
    session.on_connected(secs(3));
    assert_eq!(sent(session.on_text(HELLO, secs(3)))["op"], 2);
}

#[test]
fn missing_hello_times_out_and_reconnects() {
    let mut session = session(1.0);
    session.on_connected(T0);
    assert!(session.on_tick(secs(29)).is_empty());
    let (close_code, delay) = disconnect(session.on_tick(secs(30)));
    assert_eq!(close_code, Some(4000));
    assert_eq!(delay, secs(1));
}

#[test]
fn garbage_and_unknown_frames_are_ignored() {
    let mut session = session(1.0);
    establish(&mut session, T0);
    assert!(session.on_text("not json at all", T0).is_empty());
    assert!(session.on_text(r#"{"op":99,"d":null}"#, T0).is_empty());
    // Known event, wrong shape: dropped, but the sequence still advances so
    // a later resume does not replay it forever.
    let bad = r#"{"op":0,"t":"MESSAGE_CREATE","s":77,"d":{"id":"1"}}"#;
    assert!(session.on_text(bad, T0).is_empty());
    assert_eq!(session.seq(), Some(77));
    assert!(session.is_active());
}

#[test]
fn frames_after_disconnect_are_ignored() {
    let mut session = session(1.0);
    establish(&mut session, T0);
    let _ = session.on_closed(None);
    assert!(session.on_text(MESSAGE_CREATE, T0).is_empty());
    assert!(session.on_tick(secs(1000)).is_empty());
    assert_eq!(session.seq(), Some(1));
}

#[test]
fn debug_output_never_contains_the_token() {
    let mut session = session(1.0);
    session.on_connected(T0);
    let actions = session.on_text(HELLO, T0);
    let rendered = format!("{session:?} {actions:?}");
    assert!(!rendered.contains(FAKE_TOKEN), "leaked: {rendered}");
}
