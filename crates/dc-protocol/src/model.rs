//! Resource models: the minimum needed to list guilds and channels and to
//! read messages.
//!
//! Timestamps stay as the ISO8601 strings Discord sends; nothing in M0 does
//! arithmetic on them, so no date-time dependency is pulled in yet.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::Snowflake;

/// Unknown fields kept verbatim, so later milestones can read data that is
/// not modelled yet without a protocol change.
pub type Extra = Map<String, Value>;

/// A (partial) user object.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct User {
    pub id: Snowflake,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub discriminator: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub bot: bool,
    /// Banner hash; only present on some user objects.
    #[serde(default)]
    pub banner: Option<String>,
    /// Banner color as an RGB integer; only present on some user objects.
    #[serde(default)]
    pub accent_color: Option<u32>,
    #[serde(default)]
    pub avatar_decoration_data: Option<AvatarDecoration>,
}

/// The avatar decoration a user has equipped.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct AvatarDecoration {
    /// Asset hash, used with the avatar-decoration-presets CDN route.
    pub asset: String,
}

impl User {
    /// The name a client would show: display name, falling back to username.
    pub fn display_name(&self) -> &str {
        match self.global_name.as_deref() {
            Some(name) if !name.is_empty() => name,
            _ => &self.username,
        }
    }
}

/// Channel type. Unknown values are preserved rather than rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChannelType {
    GuildText,
    Dm,
    GuildVoice,
    GroupDm,
    GuildCategory,
    GuildNews,
    NewsThread,
    PublicThread,
    PrivateThread,
    GuildStageVoice,
    GuildDirectory,
    GuildForum,
    GuildMedia,
    Unknown(u16),
}

impl ChannelType {
    pub const fn from_u16(value: u16) -> Self {
        match value {
            0 => Self::GuildText,
            1 => Self::Dm,
            2 => Self::GuildVoice,
            3 => Self::GroupDm,
            4 => Self::GuildCategory,
            5 => Self::GuildNews,
            10 => Self::NewsThread,
            11 => Self::PublicThread,
            12 => Self::PrivateThread,
            13 => Self::GuildStageVoice,
            14 => Self::GuildDirectory,
            15 => Self::GuildForum,
            16 => Self::GuildMedia,
            other => Self::Unknown(other),
        }
    }

    pub const fn as_u16(self) -> u16 {
        match self {
            Self::GuildText => 0,
            Self::Dm => 1,
            Self::GuildVoice => 2,
            Self::GroupDm => 3,
            Self::GuildCategory => 4,
            Self::GuildNews => 5,
            Self::NewsThread => 10,
            Self::PublicThread => 11,
            Self::PrivateThread => 12,
            Self::GuildStageVoice => 13,
            Self::GuildDirectory => 14,
            Self::GuildForum => 15,
            Self::GuildMedia => 16,
            Self::Unknown(other) => other,
        }
    }

    /// Whether messages can be read directly in a channel of this type.
    pub const fn is_text_like(self) -> bool {
        matches!(
            self,
            Self::GuildText
                | Self::Dm
                | Self::GroupDm
                | Self::GuildNews
                | Self::NewsThread
                | Self::PublicThread
                | Self::PrivateThread
                | Self::GuildVoice
                | Self::GuildStageVoice
        )
    }
}

impl Default for ChannelType {
    fn default() -> Self {
        Self::Unknown(u16::MAX)
    }
}

impl<'de> Deserialize<'de> for ChannelType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u16::deserialize(deserializer).map(Self::from_u16)
    }
}

impl Serialize for ChannelType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u16(self.as_u16())
    }
}

/// A channel: guild channel, thread, DM or group DM.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Channel {
    pub id: Snowflake,
    #[serde(rename = "type", default)]
    pub kind: ChannelType,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default)]
    pub parent_id: Option<Snowflake>,
    #[serde(default)]
    pub last_message_id: Option<Snowflake>,
    #[serde(default)]
    pub nsfw: bool,
    /// Recipients of a DM / group DM, excluding the current user.
    #[serde(default)]
    pub recipients: Vec<User>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Channel {
    /// A human-readable label: the channel name, or the recipients for DMs.
    pub fn label(&self) -> String {
        if let Some(name) = self.name.as_deref().filter(|n| !n.is_empty()) {
            return name.to_owned();
        }
        if self.recipients.is_empty() {
            return format!("channel {}", self.id);
        }
        self.recipients
            .iter()
            .map(User::display_name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Guild fields that live under `properties` when the `CLIENT_STATE_V2`
/// gateway capability is enabled.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct GuildProperties {
    #[serde(default)]
    pub id: Option<Snowflake>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A gateway guild object as found in READY and GUILD_CREATE.
///
/// Two shapes exist: without the `CLIENT_STATE_V2` capability the guild
/// fields (`id`, `name`, ...) are merged into this object; with it they sit
/// under `properties`. Both are accepted; use [`Guild::id`] / [`Guild::name`].
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct Guild {
    #[serde(default, rename = "id")]
    pub raw_id: Option<Snowflake>,
    #[serde(default, rename = "name")]
    pub raw_name: Option<String>,
    #[serde(default, rename = "icon")]
    pub raw_icon: Option<String>,
    #[serde(default)]
    pub properties: Option<GuildProperties>,
    #[serde(default)]
    pub unavailable: bool,
    #[serde(default)]
    pub member_count: Option<u64>,
    #[serde(default)]
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub threads: Vec<Channel>,
}

impl Guild {
    // ASSUMPTION: with CLIENT_STATE_V2 the guild `id` is present at the top
    // level and/or inside `properties`; the docs only say `properties` is "an
    // otherwise-normal guild object". We read whichever exists.
    pub fn id(&self) -> Option<Snowflake> {
        self.raw_id
            .or_else(|| self.properties.as_ref().and_then(|p| p.id))
    }

    pub fn name(&self) -> Option<&str> {
        self.raw_name
            .as_deref()
            .or_else(|| self.properties.as_ref().and_then(|p| p.name.as_deref()))
    }

    /// Icon hash, if the guild has an icon.
    pub fn icon(&self) -> Option<&str> {
        self.raw_icon
            .as_deref()
            .or_else(|| self.properties.as_ref().and_then(|p| p.icon.as_deref()))
    }
}

/// A file attached to a message.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Attachment {
    pub id: Snowflake,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub content_type: Option<String>,
}

/// A message, as returned by REST and by MESSAGE_CREATE (which adds
/// `guild_id` and friends).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Message {
    pub id: Snowflake,
    pub channel_id: Snowflake,
    /// Only present on gateway events for guild messages.
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    pub author: User,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub edited_timestamp: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: u16,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub mentions: Vec<User>,
    #[serde(default)]
    pub mention_everyone: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub tts: bool,
    #[serde(default)]
    pub flags: u64,
    #[serde(default)]
    pub webhook_id: Option<Snowflake>,
    /// Embeds, components, reactions, references, ... (not modelled in M0).
    #[serde(flatten)]
    pub extra: Extra,
}

/// MESSAGE_UPDATE data.
///
/// The docs describe it as a full message object, but everything except the
/// IDs is optional here so a partial update can never break the connection.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct MessageUpdate {
    pub id: Snowflake,
    pub channel_id: Snowflake,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(default)]
    pub author: Option<User>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub edited_timestamp: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// MESSAGE_DELETE data.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct MessageDelete {
    pub id: Snowflake,
    pub channel_id: Snowflake,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
}

/// The READY dispatch: everything needed to start using the session.
///
/// Only the fields M0 needs are modelled; the (many) others are ignored.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Ready {
    #[serde(default)]
    pub v: u8,
    pub user: User,
    pub session_id: String,
    /// WebSocket URL to use when resuming this session.
    #[serde(default)]
    pub resume_gateway_url: Option<String>,
    #[serde(default)]
    pub session_type: Option<String>,
    #[serde(default)]
    pub guilds: Vec<Guild>,
    #[serde(default)]
    pub private_channels: Vec<Channel>,
}

/// Response of `GET /gateway`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct GatewayInfo {
    pub url: String,
}

/// JSON body of an API error response.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct ApiErrorBody {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub code: Option<u64>,
    /// Present on 429 responses: seconds to wait.
    #[serde(default)]
    pub retry_after: Option<f64>,
    #[serde(default)]
    pub global: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_channel_type_round_trips() {
        let channel: Channel =
            serde_json::from_str(r#"{"id":"1","type":9999,"brand_new_field":{"a":1}}"#).unwrap();
        assert_eq!(channel.kind, ChannelType::Unknown(9999));
        assert!(channel.extra.contains_key("brand_new_field"));
        let back = serde_json::to_value(&channel).unwrap();
        assert_eq!(back["type"], 9999);
        assert_eq!(back["brand_new_field"]["a"], 1);
    }

    #[test]
    fn channel_without_type_still_parses() {
        let channel: Channel = serde_json::from_str(r#"{"id":"1"}"#).unwrap();
        assert!(matches!(channel.kind, ChannelType::Unknown(_)));
        assert_eq!(channel.label(), "channel 1");
    }

    #[test]
    fn display_name_falls_back_to_username() {
        let user: User =
            serde_json::from_str(r#"{"id":"1","username":"fake_user","global_name":null}"#)
                .unwrap();
        assert_eq!(user.display_name(), "fake_user");
    }

    #[test]
    fn guild_reads_both_shapes() {
        let merged: Guild = serde_json::from_str(r#"{"id":"5","name":"Merged"}"#).unwrap();
        let nested: Guild =
            serde_json::from_str(r#"{"properties":{"id":"5","name":"Nested"}}"#).unwrap();
        assert_eq!(merged.id(), Some(Snowflake(5)));
        assert_eq!(merged.name(), Some("Merged"));
        assert_eq!(nested.id(), Some(Snowflake(5)));
        assert_eq!(nested.name(), Some("Nested"));
    }
}
