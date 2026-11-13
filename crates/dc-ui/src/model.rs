//! Pure view-models: `UI = f(store, overlay)` (blueprint §4).
//!
//! Everything that decides *what* is shown lives here and is unit-tested;
//! `view` only draws these structs.

use dc_core::overlay::{Color, LocalOverlay};
use dc_protocol::Snowflake;
use dc_protocol::cdn;
use dc_protocol::model::{Channel, ChannelType, Message, User};
use dc_store::Store;

/// What the channel column is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Direct messages.
    Home,
    Guild(Snowflake),
}

/// Where an image comes from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ImageRef {
    /// A file on this machine (local cosmetics).
    Local(String),
    /// A Discord CDN URL, downloaded by the backend.
    Remote(String),
}

/// Up to two characters to draw when there is no image.
pub fn initials(name: &str) -> String {
    let letters: String = name
        .split_whitespace()
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .collect();
    if letters.is_empty() {
        "?".to_owned()
    } else {
        letters.to_uppercase()
    }
}

// ---------------------------------------------------------------------------
// Server rail
// ---------------------------------------------------------------------------

/// One server icon in the left rail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailItem {
    pub guild_id: Snowflake,
    pub name: String,
    pub icon: Option<ImageRef>,
    pub initials: String,
    pub unavailable: bool,
}

pub fn rail(store: &Store) -> Vec<RailItem> {
    store
        .guilds()
        .iter()
        .map(|guild| RailItem {
            guild_id: guild.id,
            name: guild.name.clone(),
            icon: guild
                .icon
                .as_deref()
                .map(|icon| ImageRef::Remote(cdn::guild_icon_url(guild.id, icon, 96))),
            initials: initials(&guild.name),
            unavailable: guild.unavailable,
        })
        .collect()
}

/// Heading above the channel column.
pub fn scope_title(store: &Store, scope: Scope) -> String {
    match scope {
        Scope::Home => "Direct Messages".to_owned(),
        Scope::Guild(id) => match store.guild(id) {
            Some(guild) if !guild.name.trim().is_empty() => guild.name.clone(),
            Some(_) => "(unnamed server)".to_owned(),
            None => "Unknown server".to_owned(),
        },
    }
}

// ---------------------------------------------------------------------------
// Channel column
// ---------------------------------------------------------------------------

/// One line of the channel column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarRow {
    pub kind: RowKind,
    pub label: String,
    /// Set for rows that open a channel when clicked.
    pub channel_id: Option<Snowflake>,
    /// Avatar for DM rows.
    pub avatar: Option<ImageRef>,
    /// Second line for DM rows (e.g. member count of a group).
    pub subtitle: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Category,
    /// A text-like guild channel.
    Channel,
    /// A voice or stage channel (its text chat can be opened).
    Voice,
    /// A channel that cannot be opened here yet (forum, directory, ...).
    Inert,
    /// A DM or group DM.
    Dm,
}

fn guild_channel_row(channel: &Channel) -> SidebarRow {
    let voice = matches!(
        channel.kind,
        ChannelType::GuildVoice | ChannelType::GuildStageVoice
    );
    let openable = channel.kind.is_text_like();
    SidebarRow {
        kind: match (openable, voice) {
            (true, true) => RowKind::Voice,
            (true, false) => RowKind::Channel,
            (false, _) => RowKind::Inert,
        },
        label: channel.label(),
        channel_id: openable.then_some(channel.id),
        avatar: None,
        subtitle: None,
    }
}

fn is_thread(kind: ChannelType) -> bool {
    matches!(
        kind,
        ChannelType::NewsThread | ChannelType::PublicThread | ChannelType::PrivateThread
    )
}

fn dm_row(channel: &Channel, overlay: &LocalOverlay, me: Option<Snowflake>) -> SidebarRow {
    let (label, avatar, subtitle) = match (channel.kind, channel.recipients.as_slice()) {
        (ChannelType::Dm, [user]) => {
            let view = overlay.view_user(user, me);
            (view.name, Some(user_avatar(user, overlay, me, 64)), None)
        }
        _ => (
            channel.label(),
            None,
            Some(format!("{} Members", channel.recipients.len() + 1)),
        ),
    };
    SidebarRow {
        kind: RowKind::Dm,
        label,
        channel_id: Some(channel.id),
        avatar,
        subtitle,
    }
}

/// Rows of the channel column for the given scope.
///
/// Home: DMs, most recently active first. Guild: uncategorized channels,
/// then each category followed by its channels, ordered by `position`.
/// Threads are not listed yet.
pub fn sidebar(store: &Store, overlay: &LocalOverlay, scope: Scope) -> Vec<SidebarRow> {
    let me = store.current_user().map(|u| u.id);
    match scope {
        Scope::Home => {
            let mut channels: Vec<&Channel> = store
                .private_channels()
                .iter()
                .filter_map(|id| store.channel(*id))
                .collect();
            channels.sort_by_key(|c| std::cmp::Reverse(c.last_message_id.unwrap_or(c.id)));
            channels
                .into_iter()
                .map(|c| dm_row(c, overlay, me))
                .collect()
        }
        Scope::Guild(guild_id) => {
            let Some(guild) = store.guild(guild_id) else {
                return Vec::new();
            };
            let mut channels: Vec<&Channel> = guild
                .channels
                .iter()
                .filter_map(|id| store.channel(*id))
                .filter(|c| !is_thread(c.kind))
                .collect();
            channels.sort_by_key(|c| (c.position.unwrap_or(i64::MAX), c.id));

            let is_category = |c: &Channel| c.kind == ChannelType::GuildCategory;
            let category_ids: Vec<Snowflake> = channels
                .iter()
                .filter(|c| is_category(c))
                .map(|c| c.id)
                .collect();

            let mut rows = Vec::new();
            for channel in &channels {
                let orphan = channel.parent_id.is_none_or(|p| !category_ids.contains(&p));
                if !is_category(channel) && orphan {
                    rows.push(guild_channel_row(channel));
                }
            }
            for category in channels.iter().filter(|c| is_category(c)) {
                rows.push(SidebarRow {
                    kind: RowKind::Category,
                    label: category.label().to_uppercase(),
                    channel_id: None,
                    avatar: None,
                    subtitle: None,
                });
                for channel in &channels {
                    if channel.parent_id == Some(category.id) && !is_category(channel) {
                        rows.push(guild_channel_row(channel));
                    }
                }
            }
            rows
        }
    }
}

/// Title for the message pane.
pub fn channel_title(
    store: &Store,
    overlay: &LocalOverlay,
    channel_id: Option<Snowflake>,
) -> String {
    let me = store.current_user().map(|u| u.id);
    match channel_id.and_then(|id| store.channel(id)) {
        Some(channel) if matches!(channel.kind, ChannelType::Dm | ChannelType::GroupDm) => {
            dm_row(channel, overlay, me).label
        }
        Some(channel) => format!("# {}", channel.label()),
        None => "Select a channel".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Messages from the same author within this window share one header.
const GROUP_WINDOW_MS: u64 = 7 * 60 * 1000;
const REPLY_PREVIEW_CHARS: usize = 90;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyPreview {
    pub author: String,
    pub author_color: Option<Color>,
    pub content: String,
}

/// One message as drawn in the message list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageRow {
    pub id: Snowflake,
    pub author_id: Snowflake,
    pub author: String,
    /// Local name color from the overlay, if any.
    pub author_color: Option<Color>,
    pub avatar: ImageRef,
    pub is_bot: bool,
    /// Local time, e.g. `1:20 PM`.
    pub time: String,
    pub content: String,
    pub edited: bool,
    /// File names of attachments.
    pub attachments: Vec<String>,
    /// False for follow-up messages grouped under the previous header.
    pub show_header: bool,
    pub reply: Option<ReplyPreview>,
}

/// `H:MM AM/PM` for a Unix time in milliseconds, shifted by a UTC offset.
pub fn format_time(unix_ms: u64, utc_offset_minutes: i32) -> String {
    let minutes = i64::try_from(unix_ms / 60_000).unwrap_or(0) + i64::from(utc_offset_minutes);
    let of_day = minutes.rem_euclid(24 * 60);
    let (hour, minute) = (of_day / 60, of_day % 60);
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let hour12 = match hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{hour12}:{minute:02} {suffix}")
}

/// Replaces Discord's inline markup with readable text: `<@id>` -> `@name`,
/// `<#id>` -> `#channel`, `<@&id>` -> `@role`, `<:name:id>` -> `:name:`.
pub fn readable_content(
    content: &str,
    store: &Store,
    overlay: &LocalOverlay,
    mentions: &[User],
) -> String {
    let me = store.current_user().map(|u| u.id);
    let user_name = |id: Snowflake| -> String {
        mentions
            .iter()
            .find(|u| u.id == id)
            .or_else(|| store.user(id))
            .map_or_else(
                || "unknown-user".to_owned(),
                |user| overlay.view_user(user, me).name,
            )
    };

    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find('>') else {
            rest = tail;
            break;
        };
        let inner = &tail[1..end];
        let id_of = |text: &str| text.parse::<Snowflake>().ok();
        let replacement = if let Some(id) = inner.strip_prefix("@&").and_then(id_of) {
            let _ = id;
            Some("@role".to_owned())
        } else if let Some(id) = inner
            .strip_prefix("@!")
            .or_else(|| inner.strip_prefix('@'))
            .and_then(id_of)
        {
            Some(format!("@{}", user_name(id)))
        } else if let Some(id) = inner.strip_prefix('#').and_then(id_of) {
            Some(match store.channel(id) {
                Some(channel) => format!("#{}", channel.label()),
                None => "#unknown-channel".to_owned(),
            })
        } else if let Some(emoji) = inner.strip_prefix("a:").or_else(|| inner.strip_prefix(':')) {
            match emoji.split_once(':') {
                Some((name, id)) if !name.is_empty() && id_of(id).is_some() => {
                    Some(format!(":{name}:"))
                }
                _ => None,
            }
        } else {
            None
        };
        match replacement {
            Some(text) => {
                out.push_str(&text);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('<');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn truncate_chars(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or_default();
    if line.chars().count() <= max {
        line.to_owned()
    } else {
        let cut: String = line.chars().take(max).collect();
        format!("{cut}…")
    }
}

fn reply_preview(
    message: &Message,
    store: &Store,
    overlay: &LocalOverlay,
    me: Option<Snowflake>,
) -> Option<ReplyPreview> {
    let value = message.extra.get("referenced_message")?;
    if value.is_null() {
        return None;
    }
    let referenced: Message = serde_json::from_value(value.clone()).ok()?;
    let author = overlay.view_user(&referenced.author, me);
    let content = readable_content(&referenced.content, store, overlay, &referenced.mentions);
    Some(ReplyPreview {
        author: author.name,
        author_color: author.name_color,
        content: if content.trim().is_empty() {
            "Click to see attachment".to_owned()
        } else {
            truncate_chars(&content, REPLY_PREVIEW_CHARS)
        },
    })
}

/// Messages of a channel, oldest first, with the overlay applied to authors.
pub fn messages(
    store: &Store,
    overlay: &LocalOverlay,
    channel_id: Snowflake,
    utc_offset_minutes: i32,
) -> Vec<MessageRow> {
    let me = store.current_user().map(|u| u.id);
    let mut rows: Vec<MessageRow> = Vec::new();
    let mut previous: Option<(Snowflake, u64)> = None;

    for message in store.messages(channel_id) {
        let author = overlay.view_user(&message.author, me);
        let sent_ms = cdn::snowflake_unix_ms(message.id);
        let reply = reply_preview(message, store, overlay, me);
        let grouped = reply.is_none()
            && previous.is_some_and(|(author_id, at)| {
                author_id == message.author.id && sent_ms.saturating_sub(at) <= GROUP_WINDOW_MS
            });
        previous = Some((message.author.id, sent_ms));

        let content = if message.content.is_empty() && message.attachments.is_empty() {
            "(embed, sticker or system message)".to_owned()
        } else {
            readable_content(&message.content, store, overlay, &message.mentions)
        };
        rows.push(MessageRow {
            id: message.id,
            author_id: message.author.id,
            author: author.name,
            author_color: author.name_color,
            avatar: user_avatar(&message.author, overlay, me, 64),
            is_bot: message.author.bot,
            time: format_time(sent_ms, utc_offset_minutes),
            content,
            edited: message.edited_timestamp.is_some(),
            attachments: message
                .attachments
                .iter()
                .map(|a| a.filename.clone())
                .collect(),
            show_header: !grouped,
            reply,
        });
    }
    rows
}

// ---------------------------------------------------------------------------
// Profiles
// ---------------------------------------------------------------------------

fn user_avatar(user: &User, overlay: &LocalOverlay, me: Option<Snowflake>, size: u16) -> ImageRef {
    match overlay.view_user(user, me).avatar {
        Some(path) => ImageRef::Local(path),
        None => ImageRef::Remote(cdn::avatar_url(user.id, user.avatar.as_deref(), size)),
    }
}

/// A local decoration setting is a file path if it looks like one,
/// otherwise the asset hash of a catalog decoration.
fn decoration_ref(value: &str) -> ImageRef {
    if value.contains(['/', '\\', '.']) {
        ImageRef::Local(value.to_owned())
    } else {
        ImageRef::Remote(cdn::avatar_decoration_url(value, 96))
    }
}

/// A user's profile card: what Discord sent, with local cosmetics on top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileView {
    pub user_id: Snowflake,
    pub name: String,
    pub username: String,
    pub name_color: Option<Color>,
    pub avatar: ImageRef,
    pub decoration: Option<ImageRef>,
    pub banner: Option<ImageRef>,
    /// Banner color when there is no banner image.
    pub banner_color: Option<Color>,
    /// Card tint; with `banner_color` it forms a gradient.
    pub profile_color: Option<Color>,
    pub bio: Option<String>,
    pub is_bot: bool,
    pub is_self: bool,
    /// True if local cosmetics changed anything.
    pub customized: bool,
}

pub fn profile_of(user: &User, store: &Store, overlay: &LocalOverlay) -> ProfileView {
    let me = store.current_user().map(|u| u.id);
    let view = overlay.view_user(user, me);
    ProfileView {
        user_id: user.id,
        name: view.name,
        username: user.username.clone(),
        name_color: view.name_color,
        avatar: user_avatar(user, overlay, me, 128),
        decoration: view
            .avatar_decoration
            .as_deref()
            .map(decoration_ref)
            .or_else(|| {
                user.avatar_decoration_data
                    .as_ref()
                    .map(|d| ImageRef::Remote(cdn::avatar_decoration_url(&d.asset, 96)))
            }),
        banner: view.banner.map(ImageRef::Local).or_else(|| {
            user.banner
                .as_deref()
                .map(|hash| ImageRef::Remote(cdn::banner_url(user.id, hash, 480)))
        }),
        banner_color: view.accent_color.or(user.accent_color.map(Color)),
        profile_color: view.profile_color,
        bio: view.bio,
        is_bot: user.bot,
        is_self: me == Some(user.id),
        customized: view.customized,
    }
}

/// Profile card for any user the store has seen.
pub fn profile(store: &Store, overlay: &LocalOverlay, user_id: Snowflake) -> Option<ProfileView> {
    store
        .user(user_id)
        .map(|user| profile_of(user, store, overlay))
}

/// The logged-in user's own card (bottom-left user bar).
pub fn own_profile(store: &Store, overlay: &LocalOverlay) -> Option<ProfileView> {
    store
        .current_user()
        .map(|user| profile_of(user, store, overlay))
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

    const GUILD: Snowflake = Snowflake(200_000_000_000_000_001);
    const GENERAL: Snowflake = Snowflake(300_000_000_000_000_002);
    const ME: Snowflake = Snowflake(100_000_000_000_000_001);
    const FRIEND: Snowflake = Snowflake(100_000_000_000_000_002);

    fn event(text: &str) -> DispatchEvent {
        match GatewayReceive::parse(text).unwrap() {
            GatewayReceive::Dispatch { name, data, .. } => {
                DispatchEvent::decode(&name, data).unwrap()
            }
            other => panic!("not a dispatch: {other:?}"),
        }
    }

    fn store() -> Store {
        let mut store = Store::new();
        store.apply(&event(fixture!("gateway/ready.json")));
        store.insert_history(
            GENERAL,
            serde_json::from_str(fixture!("rest/messages.json")).unwrap(),
        );
        store
    }

    fn none() -> LocalOverlay {
        LocalOverlay::default()
    }

    #[test]
    fn initials_from_names() {
        assert_eq!(initials("Fake Friends Server"), "FF");
        assert_eq!(initials("solo"), "S");
        assert_eq!(initials("  ˚.⋆ chris"), "C");
        assert_eq!(initials(""), "?");
    }

    #[test]
    fn rail_lists_guilds() {
        let rail = rail(&store());
        assert_eq!(rail.len(), 2);
        assert_eq!(rail[0].name, "Fake Friends Server");
        assert_eq!(rail[0].initials, "FF");
        assert_eq!(rail[0].icon, None);
        assert!(rail[1].unavailable);
        assert_eq!(
            scope_title(&store(), Scope::Guild(GUILD)),
            "Fake Friends Server"
        );
        assert_eq!(scope_title(&store(), Scope::Home), "Direct Messages");
    }

    #[test]
    fn guild_sidebar_groups_channels_under_categories() {
        let rows = sidebar(&store(), &none(), Scope::Guild(GUILD));
        let summary: Vec<(RowKind, &str, bool)> = rows
            .iter()
            .map(|r| (r.kind, r.label.as_str(), r.channel_id.is_some()))
            .collect();
        assert_eq!(
            summary,
            [
                (RowKind::Voice, "Voice", true),
                (RowKind::Inert, "channel-type-from-the-future", false),
                (RowKind::Category, "TEXT CHANNELS", false),
                (RowKind::Channel, "general", true),
            ]
        );
        assert!(sidebar(&store(), &none(), Scope::Guild(Snowflake(1))).is_empty());
    }

    #[test]
    fn home_sidebar_lists_dms_most_recent_first_with_avatars() {
        let rows = sidebar(&store(), &none(), Scope::Home);
        assert_eq!(rows.len(), 2);
        // The direct DM's last_message_id is newer than the group DM's
        // channel ID (the fallback when it has no messages).
        assert_eq!(rows[0].label, "Fake Friend");
        assert_eq!(rows[0].subtitle, None);
        assert!(matches!(rows[0].avatar, Some(ImageRef::Remote(_))));
        assert_eq!(rows[1].label, "Fake Friend, other_fake");
        assert_eq!(rows[1].subtitle.as_deref(), Some("3 Members"));
        assert_eq!(rows[1].avatar, None);
        assert!(rows.iter().all(|r| r.kind == RowKind::Dm));
    }

    #[test]
    fn times_are_local_12_hour() {
        // 2026-01-02T03:04:05Z
        let ms = 1_767_323_045_000;
        assert_eq!(format_time(ms, 0), "3:04 AM");
        assert_eq!(format_time(ms, -5 * 60), "10:04 PM");
        assert_eq!(format_time(ms, 9 * 60 + 30), "12:34 PM");
        assert_eq!(format_time(ms, -3 * 60 - 4), "12:00 AM");
    }

    #[test]
    fn markup_becomes_readable_text() {
        let store = store();
        let text = "hi <@100000000000000002> and <@!100000000000000003>, see \
                    <#300000000000000002> <#42> <@&7> <:pog:123> <a:dance:456> 1 < 2 <notmarkup>";
        assert_eq!(
            readable_content(text, &store, &none(), &[]),
            "hi @Fake Friend and @other_fake, see #general #unknown-channel @role :pog: :dance: \
             1 < 2 <notmarkup>"
        );
        assert_eq!(readable_content("a < b", &store, &none(), &[]), "a < b");
        assert_eq!(readable_content("hé <", &store, &none(), &[]), "hé <");
        assert_eq!(
            readable_content("<#300000000000000002> then <", &store, &none(), &[]),
            "#general then <"
        );
        assert_eq!(
            readable_content("<@999>", &store, &none(), &[]),
            "@unknown-user"
        );
    }

    #[test]
    fn message_rows_group_and_apply_the_overlay() {
        let overlay = LocalOverlay::from_json(
            r##"{"users":{"100000000000000002":{"display_name":"bestie","name_color":"#00ccff"}}}"##,
        )
        .unwrap();
        let rows = messages(&store(), &overlay, GENERAL, 0);
        assert_eq!(rows.len(), 3);
        // Oldest first: webhook, friend, owner.
        assert_eq!(rows[0].content, "(embed, sticker or system message)");
        assert!(rows[0].is_bot);
        assert_eq!(rows[1].author, "bestie");
        assert_eq!(rows[1].author_color, Some(Color(0x00ccff)));
        assert_eq!(rows[1].author_id, FRIEND);
        assert!(rows[1].edited);
        assert!(
            rows.iter().all(|r| r.show_header),
            "three different authors"
        );
        assert!(rows.iter().all(|r| r.time.ends_with('M')));
        assert!(
            matches!(&rows[1].avatar, ImageRef::Remote(url) if url.contains("/embed/avatars/"))
        );
    }

    #[test]
    fn consecutive_messages_from_one_author_are_grouped() {
        let mut store = store();
        let template = store.messages(GENERAL)[2].clone();
        let base = template.id.get();
        // +1 minute: grouped. +20 minutes: new header.
        for (offset_ms, content) in [(60_000u64, "second"), (20 * 60_000, "much later")] {
            let mut message = template.clone();
            message.id = Snowflake(base + (offset_ms << 22));
            message.content = content.to_owned();
            store.apply(&DispatchEvent::MessageCreate(Box::new(message)));
        }
        let rows = messages(&store, &none(), GENERAL, 0);
        let last: Vec<(&str, bool)> = rows[2..]
            .iter()
            .map(|r| (r.content.as_str(), r.show_header))
            .collect();
        assert_eq!(
            last,
            [
                ("newest message", true),
                ("second", false),
                ("much later", true)
            ]
        );
    }

    #[test]
    fn replies_show_a_preview_and_start_a_new_group() {
        let mut store = store();
        let mut reply = store.messages(GENERAL)[2].clone();
        reply.id = Snowflake(reply.id.get() + (1000 << 22));
        reply.content = "replying".to_owned();
        let referenced = serde_json::to_value(&store.messages(GENERAL)[1]).unwrap();
        reply
            .extra
            .insert("referenced_message".to_owned(), referenced);
        store.apply(&DispatchEvent::MessageCreate(Box::new(reply)));

        let rows = messages(&store, &none(), GENERAL, 0);
        let row = rows.last().unwrap();
        assert!(row.show_header, "a reply never joins the previous group");
        let preview = row.reply.as_ref().unwrap();
        assert_eq!(preview.author, "Fake Friend");
        assert_eq!(preview.content, "hello from a fixture");
        // A null referenced_message (deleted original) is simply no preview.
        assert_eq!(rows[1].reply, None);
    }

    #[test]
    fn profile_combines_server_data_and_local_cosmetics() {
        let overlay = LocalOverlay::from_json(
            r##"{
                "me": {
                    "display_name": "me but cooler",
                    "accent_color": "#ff66aa",
                    "profile_color": "#221133",
                    "banner": "/home/fake/banner.png",
                    "avatar_decoration": "a_fakeasset",
                    "bio": "local bio"
                },
                "users": {"100000000000000002": {"avatar_decoration": "/home/fake/decor.png"}}
            }"##,
        )
        .unwrap();
        let store = store();

        let mine = own_profile(&store, &overlay).unwrap();
        assert!(mine.is_self && mine.customized);
        assert_eq!(mine.name, "me but cooler");
        assert_eq!(mine.username, "fake_owner");
        assert_eq!(
            mine.banner,
            Some(ImageRef::Local("/home/fake/banner.png".into()))
        );
        assert_eq!(mine.banner_color, Some(Color(0xff66aa)));
        assert_eq!(mine.profile_color, Some(Color(0x221133)));
        assert_eq!(mine.bio.as_deref(), Some("local bio"));
        // A bare asset hash means a catalog decoration from the CDN.
        assert!(
            matches!(&mine.decoration, Some(ImageRef::Remote(url)) if url.contains("avatar-decoration-presets/a_fakeasset"))
        );

        let friend = profile(&store, &overlay, FRIEND).unwrap();
        assert!(!friend.is_self);
        assert_eq!(
            friend.decoration,
            Some(ImageRef::Local("/home/fake/decor.png".into()))
        );
        assert_eq!(friend.banner, None);

        let plain = profile(&store, &none(), ME).unwrap();
        assert!(!plain.customized);
        assert_eq!(plain.name, "Fake Owner");
        assert_eq!(plain.decoration, None);
        assert_eq!(profile(&store, &none(), Snowflake(1)), None);
    }

    #[test]
    fn server_side_cosmetics_are_shown_when_present() {
        let user: User = serde_json::from_str(
            r#"{"id":"100000000000000009","username":"decorated","avatar":"abc",
                "banner":"bannerhash","accent_color":1193046,
                "avatar_decoration_data":{"asset":"a_real","sku_id":"1"}}"#,
        )
        .unwrap();
        let view = profile_of(&user, &store(), &none());
        assert!(
            matches!(&view.avatar, ImageRef::Remote(url) if url.contains("/avatars/100000000000000009/abc.png"))
        );
        assert!(matches!(&view.banner, Some(ImageRef::Remote(url)) if url.contains("/banners/")));
        assert!(matches!(&view.decoration, Some(ImageRef::Remote(url)) if url.contains("a_real")));
        assert_eq!(view.banner_color, Some(Color(0x123456)));
    }

    #[test]
    fn channel_titles() {
        let store = store();
        assert_eq!(channel_title(&store, &none(), Some(GENERAL)), "# general");
        assert_eq!(channel_title(&store, &none(), None), "Select a channel");
        assert_eq!(
            channel_title(&store, &none(), Some(Snowflake(300_000_000_000_000_101))),
            "Fake Friend"
        );
    }
}
