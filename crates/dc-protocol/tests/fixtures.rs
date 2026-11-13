//! Parser tests against the sanitized fixtures in `/fixtures`.

use dc_protocol::gateway::{DispatchEvent, GatewayReceive, Hello};
use dc_protocol::model::{ApiErrorBody, ChannelType, GatewayInfo, Message};
use dc_protocol::{ProtocolError, Snowflake};

macro_rules! fixture {
    ($path:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/",
            $path
        ))
    };
}

fn dispatch(text: &str) -> (Option<u64>, DispatchEvent) {
    match GatewayReceive::parse(text).expect("valid envelope") {
        GatewayReceive::Dispatch { seq, name, data } => (
            seq,
            DispatchEvent::decode(&name, data).expect("valid event"),
        ),
        other => panic!("expected dispatch, got {other:?}"),
    }
}

#[test]
fn control_opcodes() {
    assert_eq!(
        GatewayReceive::parse(fixture!("gateway/hello.json")).unwrap(),
        GatewayReceive::Hello(Hello {
            heartbeat_interval: 41250
        })
    );
    assert_eq!(
        GatewayReceive::parse(fixture!("gateway/heartbeat_ack.json")).unwrap(),
        GatewayReceive::HeartbeatAck
    );
    assert_eq!(
        GatewayReceive::parse(fixture!("gateway/heartbeat_request.json")).unwrap(),
        GatewayReceive::Heartbeat
    );
    assert_eq!(
        GatewayReceive::parse(fixture!("gateway/reconnect.json")).unwrap(),
        GatewayReceive::Reconnect
    );
    assert_eq!(
        GatewayReceive::parse(fixture!("gateway/invalid_session.json")).unwrap(),
        GatewayReceive::InvalidSession { resumable: false }
    );
    assert_eq!(
        GatewayReceive::parse(fixture!("gateway/invalid_session_resumable.json")).unwrap(),
        GatewayReceive::InvalidSession { resumable: true }
    );
}

#[test]
fn ready_lists_guilds_and_channels() {
    let (seq, event) = dispatch(fixture!("gateway/ready.json"));
    assert_eq!(seq, Some(1));
    let DispatchEvent::Ready(ready) = event else {
        panic!("expected READY");
    };

    assert_eq!(ready.v, 9);
    assert_eq!(ready.user.id, Snowflake(100_000_000_000_000_001));
    assert_eq!(ready.user.display_name(), "Fake Owner");
    assert_eq!(ready.session_id, "fakesession0000000000000000000001");
    assert_eq!(
        ready.resume_gateway_url.as_deref(),
        Some("wss://gateway-resume.example.invalid")
    );
    assert_eq!(ready.session_type.as_deref(), Some("normal"));

    assert_eq!(ready.guilds.len(), 2);
    let guild = &ready.guilds[0];
    assert_eq!(guild.id(), Some(Snowflake(200_000_000_000_000_001)));
    assert_eq!(guild.name(), Some("Fake Friends Server"));
    assert!(!guild.unavailable);
    assert_eq!(guild.member_count, Some(4));

    let kinds: Vec<_> = guild.channels.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            ChannelType::GuildCategory,
            ChannelType::GuildText,
            ChannelType::GuildVoice,
            ChannelType::Unknown(4242),
        ]
    );
    let general = &guild.channels[1];
    assert_eq!(general.label(), "general");
    assert_eq!(general.topic.as_deref(), Some("fake topic"));
    assert_eq!(general.parent_id, Some(Snowflake(300_000_000_000_000_001)));
    assert_eq!(
        general.last_message_id,
        Some(Snowflake(400_000_000_000_000_001))
    );
    // Unmodelled fields survive in `extra`.
    assert!(general.extra.contains_key("rate_limit_per_user"));
    assert_eq!(guild.threads.len(), 1);
    assert_eq!(guild.threads[0].kind, ChannelType::PublicThread);

    let unavailable = &ready.guilds[1];
    assert!(unavailable.unavailable);
    assert_eq!(unavailable.id(), Some(Snowflake(200_000_000_000_000_002)));
    assert_eq!(unavailable.name(), None);
    assert!(unavailable.channels.is_empty());

    assert_eq!(ready.private_channels.len(), 2);
    assert_eq!(ready.private_channels[0].kind, ChannelType::Dm);
    assert_eq!(ready.private_channels[0].label(), "Fake Friend");
    assert_eq!(ready.private_channels[1].kind, ChannelType::GroupDm);
    assert_eq!(ready.private_channels[1].label(), "Fake Friend, other_fake");
}

#[test]
fn ready_with_client_state_v2_shape_still_parses() {
    let (_, event) = dispatch(fixture!("gateway/ready_client_state_v2.json"));
    let DispatchEvent::Ready(ready) = event else {
        panic!("expected READY");
    };
    let guild = &ready.guilds[0];
    assert_eq!(guild.id(), Some(Snowflake(200_000_000_000_000_001)));
    assert_eq!(guild.name(), Some("Fake Friends Server"));
    assert_eq!(guild.channels[0].label(), "general");
    // Deduped DM: no inline recipients, but it must not fail.
    assert!(ready.private_channels[0].recipients.is_empty());
    assert!(
        ready.private_channels[0]
            .extra
            .contains_key("recipient_ids")
    );
}

#[test]
fn message_create() {
    let (seq, event) = dispatch(fixture!("gateway/message_create.json"));
    assert_eq!(seq, Some(2));
    let DispatchEvent::MessageCreate(message) = event else {
        panic!("expected MESSAGE_CREATE");
    };
    assert_eq!(message.id, Snowflake(400_000_000_000_000_002));
    assert_eq!(message.channel_id, Snowflake(300_000_000_000_000_002));
    assert_eq!(message.guild_id, Some(Snowflake(200_000_000_000_000_001)));
    assert_eq!(message.author.display_name(), "Fake Friend");
    assert_eq!(message.content, "hello from a fixture");
    assert_eq!(message.timestamp, "2026-01-02T03:04:05.678000+00:00");
    assert_eq!(message.edited_timestamp, None);
    assert_eq!(message.kind, 0);
    assert_eq!(message.attachments.len(), 1);
    assert_eq!(message.attachments[0].filename, "fake.png");
    assert_eq!(
        message.attachments[0].content_type.as_deref(),
        Some("image/png")
    );
    assert!(message.extra.contains_key("member"));
    assert!(message.extra.contains_key("nonce"));
}

#[test]
fn message_update_and_delete() {
    let (_, event) = dispatch(fixture!("gateway/message_update.json"));
    let DispatchEvent::MessageUpdate(update) = event else {
        panic!("expected MESSAGE_UPDATE");
    };
    assert_eq!(update.id, Snowflake(400_000_000_000_000_002));
    assert_eq!(
        update.content.as_deref(),
        Some("hello from a fixture (edited)")
    );
    assert!(update.author.is_none());
    assert!(update.edited_timestamp.is_some());

    let (_, event) = dispatch(fixture!("gateway/message_delete.json"));
    let DispatchEvent::MessageDelete(delete) = event else {
        panic!("expected MESSAGE_DELETE");
    };
    assert_eq!(delete.id, Snowflake(400_000_000_000_000_002));
    assert_eq!(delete.channel_id, Snowflake(300_000_000_000_000_002));
}

#[test]
fn resumed_and_unknown_events() {
    let (seq, event) = dispatch(fixture!("gateway/resumed.json"));
    assert_eq!(seq, Some(12));
    assert_eq!(event, DispatchEvent::Resumed);

    let (seq, event) = dispatch(fixture!("gateway/unknown_event.json"));
    assert_eq!(seq, Some(5));
    let DispatchEvent::Unknown { name, data } = &event else {
        panic!("expected unknown event");
    };
    assert_eq!(name, "SOME_EVENT_ADDED_LATER");
    assert_eq!(data["nested"]["list"][2], 3);
    assert_eq!(event.name(), "SOME_EVENT_ADDED_LATER");
}

#[test]
fn known_event_with_wrong_shape_is_a_typed_error() {
    let err = DispatchEvent::decode("MESSAGE_CREATE", serde_json::json!({"id": "1"})).unwrap_err();
    match err {
        ProtocolError::MalformedEvent { event, .. } => assert_eq!(event, "MESSAGE_CREATE"),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn rest_messages() {
    let messages: Vec<Message> = serde_json::from_str(fixture!("rest/messages.json")).unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].content, "newest message");
    assert_eq!(messages[0].guild_id, None);
    assert_eq!(messages[1].kind, 19);
    assert!(messages[1].edited_timestamp.is_some());
    assert!(messages[1].extra.contains_key("message_reference"));
    // Sparse webhook message: most fields missing, still parses.
    assert!(messages[2].author.bot);
    assert_eq!(
        messages[2].webhook_id,
        Some(Snowflake(600_000_000_000_000_001))
    );
    assert_eq!(messages[2].content, "");
    assert!(messages[2].extra.contains_key("embeds"));
}

#[test]
fn rest_misc_bodies() {
    let gateway: GatewayInfo = serde_json::from_str(fixture!("rest/gateway.json")).unwrap();
    assert_eq!(gateway.url, "wss://gateway.example.invalid");

    let limited: ApiErrorBody = serde_json::from_str(fixture!("rest/rate_limited.json")).unwrap();
    assert_eq!(limited.retry_after, Some(1.5));
    assert_eq!(limited.global, Some(false));

    let global: ApiErrorBody =
        serde_json::from_str(fixture!("rest/rate_limited_global.json")).unwrap();
    assert_eq!(global.retry_after, Some(3.0));
    assert_eq!(global.global, Some(true));

    let denied: ApiErrorBody = serde_json::from_str(fixture!("rest/missing_access.json")).unwrap();
    assert_eq!(denied.code, Some(50001));
    assert_eq!(denied.message, "Missing Access");
}
