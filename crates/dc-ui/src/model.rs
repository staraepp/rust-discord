//! Pure view-models: `UI = f(store, overlay)` (blueprint §4).

use dc_core::overlay::{Color, LocalOverlay};
use dc_protocol::Snowflake;
use dc_protocol::model::{Channel, ChannelType};
use dc_store::Store;

/// One line of the left-hand tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarRow {
    pub kind: RowKind,
    pub label: String,
    /// Set for rows that open a channel when clicked.
    pub channel_id: Option<Snowflake>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// A server name, or the "Direct messages" heading.
    Heading,
    Category,
    /// A channel whose messages can be read.
    Channel,
    /// A channel that cannot be opened here yet (forum, directory, ...).
    Inert,
}

fn channel_row(channel: &Channel) -> SidebarRow {
    let prefix = match channel.kind {
        ChannelType::GuildVoice | ChannelType::GuildStageVoice => "🔊 ",
        ChannelType::Dm | ChannelType::GroupDm => "@ ",
        _ => "# ",
    };
    let openable = channel.kind.is_text_like();
    SidebarRow {
        kind: if openable {
            RowKind::Channel
        } else {
            RowKind::Inert
        },
        label: format!("{prefix}{}", channel.label()),
        channel_id: openable.then_some(channel.id),
    }
}

fn is_thread(kind: ChannelType) -> bool {
    matches!(
        kind,
        ChannelType::NewsThread | ChannelType::PublicThread | ChannelType::PrivateThread
    )
}

/// The guild / channel tree: for every guild its uncategorized channels,
/// then each category followed by its channels, all ordered by `position`.
/// Threads are not listed yet.
pub fn sidebar(store: &Store) -> Vec<SidebarRow> {
    let mut rows = Vec::new();

    for guild in store.guilds() {
        let name = guild.name.trim();
        let label = match (name.is_empty(), guild.unavailable) {
            (true, true) => "(unavailable server)".to_owned(),
            (true, false) => "(unnamed server)".to_owned(),
            (false, true) => format!("{name} (unavailable)"),
            (false, false) => name.to_owned(),
        };
        rows.push(SidebarRow {
            kind: RowKind::Heading,
            label,
            channel_id: None,
        });

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

        // Channels with no (known) category come first.
        for channel in &channels {
            let orphan = channel.parent_id.is_none_or(|p| !category_ids.contains(&p));
            if !is_category(channel) && orphan {
                rows.push(channel_row(channel));
            }
        }
        for category in channels.iter().filter(|c| is_category(c)) {
            rows.push(SidebarRow {
                kind: RowKind::Category,
                label: category.label().to_uppercase(),
                channel_id: None,
            });
            for channel in &channels {
                if channel.parent_id == Some(category.id) && !is_category(channel) {
                    rows.push(channel_row(channel));
                }
            }
        }
    }

    if !store.private_channels().is_empty() {
        rows.push(SidebarRow {
            kind: RowKind::Heading,
            label: "Direct messages".to_owned(),
            channel_id: None,
        });
        for id in store.private_channels() {
            if let Some(channel) = store.channel(*id) {
                rows.push(channel_row(channel));
            }
        }
    }
    rows
}

/// One message as drawn in the message list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageRow {
    pub id: Snowflake,
    pub author: String,
    /// Local name color from the overlay, if any.
    pub author_color: Option<Color>,
    /// `HH:MM`, taken from the message timestamp (UTC as sent by Discord).
    pub time: String,
    pub content: String,
    pub edited: bool,
    /// File names of attachments.
    pub attachments: Vec<String>,
}

/// Messages of a channel, oldest first, with the overlay applied to authors.
pub fn messages(store: &Store, overlay: &LocalOverlay, channel_id: Snowflake) -> Vec<MessageRow> {
    let me = store.current_user().map(|u| u.id);
    store
        .messages(channel_id)
        .iter()
        .map(|message| {
            let author = overlay.view_user(&message.author, me);
            let content = if message.content.is_empty() && message.attachments.is_empty() {
                "(embed, sticker or system message)".to_owned()
            } else {
                message.content.clone()
            };
            MessageRow {
                id: message.id,
                author: author.name,
                author_color: author.name_color,
                // "2026-01-02T03:04:05..." -> "03:04"
                time: message.timestamp.get(11..16).unwrap_or_default().to_owned(),
                content,
                edited: message.edited_timestamp.is_some(),
                attachments: message
                    .attachments
                    .iter()
                    .map(|a| a.filename.clone())
                    .collect(),
            }
        })
        .collect()
}

/// The logged-in user's card at the top of the sidebar, with local cosmetics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileCard {
    pub name: String,
    pub name_color: Option<Color>,
    pub accent_color: Option<Color>,
    /// Local file path of the banner image, if one is configured.
    pub banner: Option<String>,
    pub avatar: Option<String>,
    pub avatar_decoration: Option<String>,
}

pub fn profile_card(store: &Store, overlay: &LocalOverlay) -> Option<ProfileCard> {
    let user = store.current_user()?;
    let view = overlay.view_user(user, Some(user.id));
    Some(ProfileCard {
        name: view.name,
        name_color: view.name_color,
        accent_color: view.accent_color,
        banner: view.banner,
        avatar: view.avatar,
        avatar_decoration: view.avatar_decoration,
    })
}

/// Title for the message pane.
pub fn channel_title(store: &Store, channel_id: Option<Snowflake>) -> String {
    match channel_id.and_then(|id| store.channel(id)) {
        Some(channel) => channel_row(channel).label,
        None => "Select a channel".to_owned(),
    }
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

    const GENERAL: Snowflake = Snowflake(300_000_000_000_000_002);

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
        store.apply(&event(fixture!("gateway/message_create.json")));
        store
    }

    #[test]
    fn sidebar_groups_channels_under_categories() {
        let rows = sidebar(&store());
        let summary: Vec<(RowKind, &str, bool)> = rows
            .iter()
            .map(|r| (r.kind, r.label.as_str(), r.channel_id.is_some()))
            .collect();
        assert_eq!(
            summary,
            [
                (RowKind::Heading, "Fake Friends Server", false),
                // No category: the voice channel and the unknown-type channel.
                (RowKind::Channel, "🔊 Voice", true),
                (RowKind::Inert, "# channel-type-from-the-future", false),
                (RowKind::Category, "TEXT CHANNELS", false),
                (RowKind::Channel, "# general", true),
                (RowKind::Heading, "(unavailable server)", false),
                (RowKind::Heading, "Direct messages", false),
                (RowKind::Channel, "@ Fake Friend", true),
                (RowKind::Channel, "@ Fake Friend, other_fake", true),
            ]
        );
    }

    #[test]
    fn empty_store_has_an_empty_sidebar() {
        assert!(sidebar(&Store::new()).is_empty());
        assert_eq!(profile_card(&Store::new(), &LocalOverlay::default()), None);
        assert_eq!(
            channel_title(&Store::new(), Some(GENERAL)),
            "Select a channel"
        );
    }

    #[test]
    fn message_rows_use_overlay_names_and_colors() {
        let overlay = LocalOverlay::from_json(
            r##"{"users":{"100000000000000002":{"display_name":"bestie","name_color":"#00ccff"}}}"##,
        )
        .unwrap();
        let rows = messages(&store(), &overlay, GENERAL);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].author, "bestie");
        assert_eq!(rows[0].author_color, Some(Color(0x00ccff)));
        assert_eq!(rows[0].time, "03:04");
        assert_eq!(rows[0].content, "hello from a fixture");
        assert_eq!(rows[0].attachments, ["fake.png"]);
        assert!(!rows[0].edited);

        // Without an overlay the server's name is shown.
        let plain = messages(&store(), &LocalOverlay::default(), GENERAL);
        assert_eq!(plain[0].author, "Fake Friend");
        assert_eq!(plain[0].author_color, None);
    }

    #[test]
    fn profile_card_applies_local_cosmetics_to_the_current_user() {
        let overlay = LocalOverlay::from_json(
            r##"{"me":{"accent_color":"#ff66aa","banner":"/home/fake/banner.png","display_name":"me but cooler"}}"##,
        )
        .unwrap();
        let card = profile_card(&store(), &overlay).unwrap();
        assert_eq!(card.name, "me but cooler");
        assert_eq!(card.accent_color, Some(Color(0xff66aa)));
        assert_eq!(card.banner.as_deref(), Some("/home/fake/banner.png"));

        let plain = profile_card(&store(), &LocalOverlay::default()).unwrap();
        assert_eq!(plain.name, "Fake Owner");
        assert_eq!(plain.banner, None);
    }

    #[test]
    fn channel_title_matches_the_sidebar_label() {
        assert_eq!(channel_title(&store(), Some(GENERAL)), "# general");
        assert_eq!(channel_title(&store(), None), "Select a channel");
    }
}
