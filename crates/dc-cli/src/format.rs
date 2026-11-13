//! Plain-text rendering for the terminal. Pure functions, unit-tested.

use std::fmt::Write as _;

use dc_protocol::model::{Channel, ChannelType, Message, Ready};

/// Replaces control characters so message content cannot inject terminal
/// escape sequences. Newlines and tabs are kept.
pub fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\t' => c,
            c if c.is_control() => '\u{FFFD}',
            c => c,
        })
        .collect()
}

/// `[timestamp] Author: content` plus one line per attachment.
pub fn message(message: &Message) -> String {
    // "2026-01-02T03:04:05.678000+00:00" -> "2026-01-02 03:04:05"
    let timestamp = message
        .timestamp
        .get(..19)
        .unwrap_or(&message.timestamp)
        .replace('T', " ");
    let mut out = format!(
        "[{}] {}: {}",
        sanitize(&timestamp),
        sanitize(message.author.display_name()),
        sanitize(&message.content)
    );
    if message.edited_timestamp.is_some() {
        out.push_str(" (edited)");
    }
    for attachment in &message.attachments {
        let _ = write!(
            out,
            "\n    [attachment] {} <{}>",
            sanitize(&attachment.filename),
            sanitize(&attachment.url)
        );
    }
    if message.content.is_empty() && message.attachments.is_empty() {
        out.push_str("<no text content: embed, sticker or system message>");
    }
    out
}

fn kind_tag(kind: ChannelType) -> &'static str {
    match kind {
        ChannelType::GuildText => "text",
        ChannelType::GuildVoice => "voice",
        ChannelType::GuildCategory => "category",
        ChannelType::GuildNews => "news",
        ChannelType::GuildStageVoice => "stage",
        ChannelType::GuildForum => "forum",
        ChannelType::GuildMedia => "media",
        ChannelType::GuildDirectory => "directory",
        ChannelType::NewsThread | ChannelType::PublicThread | ChannelType::PrivateThread => {
            "thread"
        }
        ChannelType::Dm => "dm",
        ChannelType::GroupDm => "group",
        ChannelType::Unknown(_) => "other",
    }
}

fn channel_line(out: &mut String, channel: &Channel) {
    let _ = writeln!(
        out,
        "    {:<20} {:<9} {}",
        channel.id,
        kind_tag(channel.kind),
        sanitize(&channel.label())
    );
}

/// The guild/channel tree from READY, with the IDs `watch`/`history` need.
pub fn overview(ready: &Ready) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Logged in as {} ({} guilds, {} private channels)",
        sanitize(ready.user.display_name()),
        ready.guilds.len(),
        ready.private_channels.len()
    );

    for guild in &ready.guilds {
        let id = guild
            .id()
            .map_or_else(|| "?".to_owned(), |id| id.to_string());
        if guild.unavailable {
            let _ = writeln!(out, "\n{id}  (unavailable)");
            continue;
        }
        let _ = writeln!(
            out,
            "\n{id}  {}",
            sanitize(guild.name().unwrap_or("(unnamed guild)"))
        );
        let mut channels: Vec<&Channel> = guild.channels.iter().collect();
        channels.sort_by_key(|c| (c.position.unwrap_or(i64::MAX), c.id));
        for channel in channels.into_iter().chain(&guild.threads) {
            channel_line(&mut out, channel);
        }
    }

    if !ready.private_channels.is_empty() {
        let _ = writeln!(out, "\nDirect messages");
        for channel in &ready.private_channels {
            channel_line(&mut out, channel);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_protocol::gateway::{DispatchEvent, GatewayReceive};

    macro_rules! fixture {
        ($path:literal) => {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/",
                $path
            ))
        };
    }

    fn event(text: &str) -> DispatchEvent {
        match GatewayReceive::parse(text).unwrap() {
            GatewayReceive::Dispatch { name, data, .. } => {
                DispatchEvent::decode(&name, data).unwrap()
            }
            other => panic!("not a dispatch: {other:?}"),
        }
    }

    #[test]
    fn sanitize_strips_escape_sequences() {
        assert_eq!(
            sanitize("a\x1b[31mred\x07\nb\tc"),
            "a\u{FFFD}[31mred\u{FFFD}\nb\tc"
        );
    }

    #[test]
    fn formats_a_message_with_attachment() {
        let DispatchEvent::MessageCreate(msg) = event(fixture!("gateway/message_create.json"))
        else {
            panic!("expected message");
        };
        assert_eq!(
            message(&msg),
            "[2026-01-02 03:04:05] Fake Friend: hello from a fixture\n    \
             [attachment] fake.png <https://cdn.example.invalid/attachments/fake.png>"
        );
    }

    #[test]
    fn formats_rest_messages() {
        let messages: Vec<Message> = serde_json::from_str(fixture!("rest/messages.json")).unwrap();
        assert_eq!(
            message(&messages[1]),
            "[2026-01-02 03:04:05] Fake Friend: hello from a fixture (edited)"
        );
        assert!(
            message(&messages[2]).ends_with("<no text content: embed, sticker or system message>")
        );
    }

    #[test]
    fn overview_lists_guilds_channels_and_dms() {
        let DispatchEvent::Ready(ready) = event(fixture!("gateway/ready.json")) else {
            panic!("expected ready");
        };
        let text = overview(&ready);
        let expected = "\
Logged in as Fake Owner (2 guilds, 2 private channels)

200000000000000001  Fake Friends Server
    300000000000000001   category  Text Channels
    300000000000000002   text      general
    300000000000000003   voice     Voice
    300000000000000004   other     channel-type-from-the-future
    300000000000000010   thread    fake thread

200000000000000002  (unavailable)

Direct messages
    300000000000000101   dm        Fake Friend
    300000000000000102   group     Fake Friend, other_fake
";
        assert_eq!(text, expected);
    }
}
