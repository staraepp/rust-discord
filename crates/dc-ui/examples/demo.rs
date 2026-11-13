//! Opens the real window with synthetic data. No account, no network.
//!
//! ```text
//! cargo run -p dc-ui --example demo
//! DC_COSMETICS_DEMO=1 cargo run -p dc-ui --example demo   # with sample cosmetics
//! ```

use dc_core::LocalOverlay;
use dc_protocol::gateway::{DispatchEvent, GatewayReceive};
use dc_protocol::model::Message;
use dc_ui::{BackendEvent, UiCommand};

macro_rules! fixture {
    ($path:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/",
            $path
        ))
    };
}

fn dispatch(text: &str) -> BackendEvent {
    match GatewayReceive::parse(text).expect("fixture envelope") {
        GatewayReceive::Dispatch { name, data, .. } => {
            let mut event = DispatchEvent::decode(&name, data).expect("fixture event");
            if let DispatchEvent::Ready(ready) = &mut event
                && let Some(guild) = ready.guilds.first_mut()
            {
                guild.emojis=serde_json::from_value(serde_json::json!([{"id":"900000000000000001","name":"demo_wave","animated":false},{"id":"900000000000000002","name":"demo_dance","animated":true}])).unwrap();
            }
            BackendEvent::Dispatch(event)
        }
        other => panic!("not a dispatch: {other:?}"),
    }
}

fn main() {
    let overlay = if std::env::var_os("DC_COSMETICS_DEMO").is_some() {
        LocalOverlay::from_json(
            r##"{
                "me": {"display_name": "me but cooler", "name_color": "#ffd166", "accent_color": "#ff66aa", "profile_color": "#522244", "bio": "A local-only demo profile."},
                "users": {"100000000000000002": {"display_name": "bestie", "name_color": "#00ccff", "profile_color": "#123344", "bio": "Synthetic friend, local cosmetics."}}
            }"##,
        )
        .expect("demo overlay")
    } else {
        LocalOverlay::default()
    };

    let (events, receiver) = futures::channel::mpsc::unbounded();
    let _ = events.unbounded_send(BackendEvent::Status("Demo data (not connected)".into()));
    let _ = events.unbounded_send(dispatch(fixture!("gateway/ready.json")));

    let options = dc_ui::UiOptions {
        overlay,
        utc_offset_minutes: 0,
        preferences: dc_core::preferences::Preferences {
            risk_accepted: true,
            ..Default::default()
        },
    };
    let sequence = std::cell::Cell::new(0u64);
    dc_ui::run(receiver, options, move |command| match command {
        UiCommand::LoadHistory(channel_id) => {
            let mut messages: Vec<Message> =
                serde_json::from_str(fixture!("rest/messages.json")).expect("fixture messages");
            for message in &mut messages {
                message.channel_id = channel_id;
            }
            let _ = events.unbounded_send(BackendEvent::History {
                channel_id,
                messages,
            });
        }
        UiCommand::LoadOlder { channel_id, .. } => {
            let _ = events.unbounded_send(BackendEvent::OlderHistory {
                channel_id,
                messages: Vec::new(),
            });
        }
        UiCommand::EditMessage { .. } | UiCommand::DeleteMessage { .. } => {
            let _ = events.unbounded_send(BackendEvent::OperationFailed(
                "Edit/delete only use real server messages; demo stays offline".into(),
            ));
        }
        UiCommand::ClearHistoryCache => {
            let _ = events.unbounded_send(BackendEvent::Status(
                "The demo does not write message history to disk".into(),
            ));
        }
        UiCommand::SaveOverlay(json) => {
            let _ = events.unbounded_send(BackendEvent::OverlaySaved { json, error: None });
        }
        UiCommand::SavePreferences(_) => {
            let _ = events.unbounded_send(BackendEvent::PreferencesSaved { error: None });
        }
        UiCommand::SubscribeMembers { guild_id, .. } => {
            let _=events.unbounded_send(BackendEvent::Dispatch(DispatchEvent::Unknown {name:"GUILD_MEMBER_LIST_UPDATE".into(),data:serde_json::json!({"guild_id":guild_id,"id":"everyone","member_count":2,"ops":[{"op":"SYNC","range":[0,99],"items":[{"group":{"id":"online","count":2}},{"member":{"user":{"id":"100000000000000001","username":"fake_owner","global_name":"Fake Owner"}},"presence":{"status":"online"}},{"member":{"user":{"id":"100000000000000002","username":"fake_friend","global_name":"Fake Friend"}},"presence":{"status":"idle"}}]}]})}));
        }
        UiCommand::SendMessage {
            channel_id,
            message,
            attachments,
        } => {
            sequence.set(sequence.get() + 1);
            if sequence.get() == 1 && std::env::var_os("DC_DEMO_FAIL_SEND").is_some() {
                let _ = events.unbounded_send(BackendEvent::SendFailed {
                    channel_id,
                    nonce: message.nonce,
                    error: "Simulated first-send failure; press Send to retry".into(),
                });
                return;
            }
            let mut sent: Message =
                serde_json::from_str::<Vec<Message>>(fixture!("rest/messages.json"))
                    .unwrap()
                    .remove(0);
            sent.channel_id = channel_id;
            sent.content = message.content;
            sent.author=serde_json::from_value(serde_json::json!({"id":"100000000000000001","username":"fake_owner","global_name":"Fake Owner"})).unwrap();
            sent.attachments = attachments
                .iter()
                .enumerate()
                .map(|(index, path)| dc_protocol::model::Attachment {
                    id: dc_protocol::Snowflake(index as u64 + 1),
                    filename: path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    size: 0,
                    url: String::new(),
                    content_type: None,
                })
                .collect();
            sent.id = dc_protocol::Snowflake(
                (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64
                    - dc_protocol::cdn::DISCORD_EPOCH_MS)
                    << 22
                    | (sequence.get() & 0x3fffff),
            );
            let _ = events.unbounded_send(BackendEvent::Sent {
                channel_id,
                nonce: message.nonce,
                message: Box::new(sent),
            });
        }
        UiCommand::FetchImage(url) => {
            let _ = events.unbounded_send(BackendEvent::ImageFailed { url });
        }
    });
}
