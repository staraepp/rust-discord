//! Normalized in-memory state: who we are, guilds, channels and messages.
//!
//! The store is a pure reducer. Gateway dispatches go in through
//! [`Store::apply`], REST history through [`Store::insert_history`]; the UI
//! reads it and never mutates server data (blueprint §4). SQLite persistence
//! is a later milestone.

use std::collections::HashMap;
pub mod members;
use dc_protocol::members::{CustomEmoji, Member, MemberUpdate, Role};

use dc_protocol::Snowflake;
use dc_protocol::gateway::DispatchEvent;
use dc_protocol::model::{Channel, Guild, Message, Ready, User};

/// How many messages are kept per channel before the oldest are dropped.
pub const MAX_MESSAGES_PER_CHANNEL: usize = 500;

/// A guild as the store sees it: identity plus the IDs of its channels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuildEntry {
    pub id: Snowflake,
    pub name: String,
    /// Icon hash, if any.
    pub icon: Option<String>,
    pub unavailable: bool,
    /// Channel and thread IDs, in the order received.
    pub channels: Vec<Snowflake>,
    pub roles: Vec<Role>,
    pub emojis: Vec<CustomEmoji>,
}

/// What a call to [`Store::apply`] changed, so the UI knows what to redraw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Nothing the store tracks.
    None,
    /// Everything (READY replaces the whole state).
    All,
    /// The guild/channel tree.
    Guilds,
    /// The messages of one channel.
    Messages(Snowflake),
}

#[derive(Clone, Debug, Default)]
pub struct Store {
    current_user: Option<User>,
    guilds: Vec<GuildEntry>,
    private_channels: Vec<Snowflake>,
    channels: HashMap<Snowflake, Channel>,
    /// Every user seen so far (self, DM recipients, message authors,
    /// mentioned users). Later sightings replace earlier ones.
    users: HashMap<Snowflake, User>,
    member_lists: HashMap<Snowflake, members::MemberList>,
    members: HashMap<Snowflake, HashMap<Snowflake, Member>>,
    /// Per channel, oldest first, unique by message ID.
    messages: HashMap<Snowflake, Vec<Message>>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current_user(&self) -> Option<&User> {
        self.current_user.as_ref()
    }

    /// Guilds in the order READY listed them.
    pub fn guilds(&self) -> &[GuildEntry] {
        &self.guilds
    }

    /// DM and group DM channel IDs.
    pub fn private_channels(&self) -> &[Snowflake] {
        &self.private_channels
    }

    pub fn channel(&self, id: Snowflake) -> Option<&Channel> {
        self.channels.get(&id)
    }

    /// A user seen in any payload so far.
    pub fn user(&self, id: Snowflake) -> Option<&User> {
        self.users.get(&id)
    }

    /// The guild a channel belongs to, if it is a guild channel.
    pub fn guild(&self, id: Snowflake) -> Option<&GuildEntry> {
        self.guilds.iter().find(|g| g.id == id)
    }

    pub fn member_list(&self, guild: Snowflake) -> Option<&members::MemberList> {
        self.member_lists.get(&guild)
    }
    pub fn guild_members(&self, guild: Snowflake) -> Vec<&Member> {
        self.members
            .get(&guild)
            .map(|m| m.values().collect())
            .unwrap_or_default()
    }
    pub fn clear_member_list(&mut self, guild: Snowflake) {
        self.member_lists.remove(&guild);
    }
    /// Messages of a channel, oldest first.
    pub fn messages(&self, channel_id: Snowflake) -> &[Message] {
        self.messages.get(&channel_id).map_or(&[], Vec::as_slice)
    }

    /// Applies one gateway dispatch.
    pub fn apply(&mut self, event: &DispatchEvent) -> Change {
        match event {
            DispatchEvent::Ready(ready) => {
                self.apply_ready(ready);
                Change::All
            }
            DispatchEvent::GuildCreate(guild) => {
                self.upsert_guild(guild);
                Change::Guilds
            }
            DispatchEvent::MessageCreate(message) => {
                self.insert_message((**message).clone());
                Change::Messages(message.channel_id)
            }
            DispatchEvent::MessageUpdate(update) => {
                let Some(message) = self
                    .messages
                    .get_mut(&update.channel_id)
                    .and_then(|list| list.iter_mut().find(|m| m.id == update.id))
                else {
                    return Change::None;
                };
                if let Some(content) = &update.content {
                    message.content.clone_from(content);
                }
                if update.edited_timestamp.is_some() {
                    message
                        .edited_timestamp
                        .clone_from(&update.edited_timestamp);
                }
                Change::Messages(update.channel_id)
            }
            DispatchEvent::MessageDelete(delete) => {
                let Some(list) = self.messages.get_mut(&delete.channel_id) else {
                    return Change::None;
                };
                let before = list.len();
                list.retain(|m| m.id != delete.id);
                if list.len() == before {
                    Change::None
                } else {
                    Change::Messages(delete.channel_id)
                }
            }
            DispatchEvent::Unknown { name, data }
                if matches!(name.as_str(), "CHANNEL_CREATE" | "CHANNEL_UPDATE") =>
            {
                let Some(id) = data
                    .get("id")
                    .and_then(|v| serde_json::from_value::<Snowflake>(v.clone()).ok())
                else {
                    return Change::None;
                };
                let mut merged = self
                    .channels
                    .get(&id)
                    .and_then(|c| serde_json::to_value(c).ok())
                    .unwrap_or(serde_json::json!({}));
                if let (Some(target), Some(incoming)) = (merged.as_object_mut(), data.as_object()) {
                    target.extend(incoming.clone());
                }
                if let Ok(channel) = serde_json::from_value::<Channel>(merged) {
                    if let Some(guild_id) = channel.guild_id {
                        if let Some(guild) = self.guilds.iter_mut().find(|g| g.id == guild_id)
                            && !guild.channels.contains(&id)
                        {
                            guild.channels.push(id);
                        }
                    } else if matches!(
                        channel.kind,
                        dc_protocol::model::ChannelType::Dm
                            | dc_protocol::model::ChannelType::GroupDm
                    ) && !self.private_channels.contains(&id)
                    {
                        self.private_channels.push(id);
                    }
                    for user in &channel.recipients {
                        self.users.insert(user.id, user.clone());
                    }
                    self.channels.insert(id, channel);
                    Change::Guilds
                } else {
                    Change::None
                }
            }
            DispatchEvent::Unknown { name, data } if name == "CHANNEL_DELETE" => {
                let Some(id) = data
                    .get("id")
                    .and_then(|v| serde_json::from_value::<Snowflake>(v.clone()).ok())
                else {
                    return Change::None;
                };
                self.channels.remove(&id);
                self.messages.remove(&id);
                self.private_channels.retain(|c| *c != id);
                for guild in &mut self.guilds {
                    guild.channels.retain(|c| *c != id);
                }
                Change::Guilds
            }
            DispatchEvent::Unknown { name, data } if name == "GUILD_DELETE" => {
                let Some(id) = data
                    .get("id")
                    .and_then(|v| serde_json::from_value::<Snowflake>(v.clone()).ok())
                else {
                    return Change::None;
                };
                if data.get("unavailable").and_then(|v| v.as_bool()) == Some(true) {
                    if let Some(guild) = self.guilds.iter_mut().find(|g| g.id == id) {
                        guild.unavailable = true;
                    }
                } else {
                    if let Some(guild) = self.guilds.iter().find(|g| g.id == id) {
                        for channel in &guild.channels {
                            self.channels.remove(channel);
                            self.messages.remove(channel);
                        }
                    }
                    self.guilds.retain(|g| g.id != id);
                    self.members.remove(&id);
                    self.member_lists.remove(&id);
                }
                Change::Guilds
            }
            DispatchEvent::Unknown { name, data } if name == "USER_UPDATE" => {
                let Some(id) = data
                    .get("id")
                    .and_then(|v| serde_json::from_value::<Snowflake>(v.clone()).ok())
                else {
                    return Change::None;
                };
                let mut merged = self
                    .users
                    .get(&id)
                    .and_then(|u| serde_json::to_value(u).ok())
                    .unwrap_or(serde_json::json!({}));
                if let (Some(target), Some(incoming)) = (merged.as_object_mut(), data.as_object()) {
                    target.extend(incoming.clone());
                }
                if let Ok(user) = serde_json::from_value::<User>(merged) {
                    if self.current_user.as_ref().is_some_and(|u| u.id == id) {
                        self.current_user = Some(user.clone());
                    }
                    self.users.insert(id, user);
                }
                Change::Guilds
            }
            DispatchEvent::Unknown { name, data } if name == "MESSAGE_DELETE_BULK" => {
                let Some(channel) = data
                    .get("channel_id")
                    .and_then(|v| serde_json::from_value::<Snowflake>(v.clone()).ok())
                else {
                    return Change::None;
                };
                if let Some(ids) = data.get("ids").and_then(|v| v.as_array())
                    && let Some(messages) = self.messages.get_mut(&channel)
                {
                    let ids: Vec<_> = ids
                        .iter()
                        .filter_map(|v| serde_json::from_value::<Snowflake>(v.clone()).ok())
                        .collect();
                    messages.retain(|m| !ids.contains(&m.id));
                }
                Change::Messages(channel)
            }
            DispatchEvent::Unknown { name, data } if name == "GUILD_MEMBER_LIST_UPDATE" => {
                let Ok(update) = serde_json::from_value::<MemberUpdate>(data.clone()) else {
                    return Change::None;
                };
                let list = self.member_lists.entry(update.guild_id).or_default();
                list.apply(&update);
                for item in list.slots.iter().flatten() {
                    if let Some(member) = &item.member {
                        self.users.insert(member.user.id, member.user.clone());
                        self.members
                            .entry(update.guild_id)
                            .or_default()
                            .insert(member.user.id, member.clone());
                    }
                }
                Change::Guilds
            }
            DispatchEvent::Unknown { name, data } if name == "GUILD_EMOJIS_UPDATE" => {
                if let (Some(id), Some(emojis)) = (
                    data.get("guild_id")
                        .and_then(|v| serde_json::from_value::<Snowflake>(v.clone()).ok()),
                    data.get("emojis")
                        .and_then(|v| serde_json::from_value::<Vec<CustomEmoji>>(v.clone()).ok()),
                ) && let Some(guild) = self.guilds.iter_mut().find(|g| g.id == id)
                {
                    guild.emojis = emojis;
                }
                Change::Guilds
            }
            DispatchEvent::Resumed | DispatchEvent::Unknown { .. } => Change::None,
        }
    }

    /// Merges a page of REST history (any order) into a channel.
    pub fn insert_history(&mut self, channel_id: Snowflake, messages: Vec<Message>) {
        for message in messages {
            if message.channel_id == channel_id {
                self.insert_message(message);
            }
        }
    }

    fn apply_ready(&mut self, ready: &Ready) {
        // A fresh READY is a full snapshot: drop everything from before.
        *self = Store::default();
        self.current_user = Some(ready.user.clone());
        self.users.insert(ready.user.id, ready.user.clone());
        for guild in &ready.guilds {
            self.upsert_guild(guild);
        }
        for channel in &ready.private_channels {
            self.private_channels.push(channel.id);
            for user in &channel.recipients {
                self.users.insert(user.id, user.clone());
            }
            self.channels.insert(channel.id, channel.clone());
        }
    }

    fn upsert_guild(&mut self, guild: &Guild) {
        let Some(id) = guild.id() else {
            return;
        };
        for member in &guild.members {
            self.users.insert(member.user.id, member.user.clone());
            self.members
                .entry(id)
                .or_default()
                .insert(member.user.id, member.clone());
        }
        let mut channel_ids = Vec::new();
        for channel in guild.channels.iter().chain(&guild.threads) {
            let mut channel = channel.clone();
            // Channels inside READY guilds often omit `guild_id`.
            channel.guild_id.get_or_insert(id);
            channel_ids.push(channel.id);
            self.channels.insert(channel.id, channel);
        }
        let entry = GuildEntry {
            id,
            name: guild.name().unwrap_or_default().to_owned(),
            icon: guild.icon().map(str::to_owned),
            unavailable: guild.unavailable,
            channels: channel_ids,
            roles: guild.roles.clone(),
            emojis: guild.emojis.clone(),
        };
        match self.guilds.iter_mut().find(|g| g.id == id) {
            Some(existing) => {
                // Drop channels that disappeared from the guild.
                for old in &existing.channels {
                    if !entry.channels.contains(old) {
                        self.channels.remove(old);
                    }
                }
                *existing = entry;
            }
            None => self.guilds.push(entry),
        }
    }

    /// Inserts keeping the list sorted by ID (snowflakes are time-ordered)
    /// and unique; an existing message with the same ID is replaced.
    fn insert_message(&mut self, message: Message) {
        self.users.insert(message.author.id, message.author.clone());
        for user in &message.mentions {
            self.users.entry(user.id).or_insert_with(|| user.clone());
        }
        let list = self.messages.entry(message.channel_id).or_default();
        match list.binary_search_by_key(&message.id, |m| m.id) {
            Ok(index) => list[index] = message,
            Err(index) => list.insert(index, message),
        }
        if list.len() > MAX_MESSAGES_PER_CHANNEL {
            let excess = list.len() - MAX_MESSAGES_PER_CHANNEL;
            list.drain(..excess);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_protocol::gateway::GatewayReceive;

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

    fn ready_store() -> Store {
        let mut store = Store::new();
        assert_eq!(
            store.apply(&event(fixture!("gateway/ready.json"))),
            Change::All
        );
        store
    }

    fn history() -> Vec<Message> {
        serde_json::from_str(fixture!("rest/messages.json")).unwrap()
    }

    #[test]
    fn ready_populates_user_guilds_and_channels() {
        let store = ready_store();
        assert_eq!(store.current_user().unwrap().username, "fake_owner");

        let guilds = store.guilds();
        assert_eq!(guilds.len(), 2);
        assert_eq!(guilds[0].name, "Fake Friends Server");
        assert_eq!(guilds[0].channels.len(), 5, "4 channels + 1 thread");
        assert!(guilds[1].unavailable);
        assert!(guilds[1].channels.is_empty());

        let general = store.channel(GENERAL).unwrap();
        assert_eq!(general.name.as_deref(), Some("general"));
        // guild_id is filled in even though the fixture channel omits it.
        assert_eq!(general.guild_id, Some(guilds[0].id));

        assert_eq!(store.private_channels().len(), 2);
        // Users are indexed: self and DM recipients.
        assert!(store.user(Snowflake(100_000_000_000_000_001)).is_some());
        assert_eq!(
            store
                .user(Snowflake(100_000_000_000_000_003))
                .unwrap()
                .username,
            "other_fake"
        );
        assert_eq!(store.guild(guilds[0].id).unwrap().icon, None);
        assert!(store.channel(store.private_channels()[0]).is_some());
    }

    #[test]
    fn a_second_ready_replaces_everything() {
        let mut store = ready_store();
        store.apply(&event(fixture!("gateway/message_create.json")));
        assert_eq!(store.messages(GENERAL).len(), 1);
        store.apply(&event(fixture!("gateway/ready.json")));
        assert!(store.messages(GENERAL).is_empty());
        assert_eq!(store.guilds().len(), 2, "no duplicated guilds");
    }

    #[test]
    fn message_lifecycle_create_update_delete() {
        let mut store = ready_store();
        assert_eq!(
            store.apply(&event(fixture!("gateway/message_create.json"))),
            Change::Messages(GENERAL)
        );
        assert_eq!(store.messages(GENERAL)[0].content, "hello from a fixture");

        assert_eq!(
            store.apply(&event(fixture!("gateway/message_update.json"))),
            Change::Messages(GENERAL)
        );
        let message = &store.messages(GENERAL)[0];
        assert_eq!(message.content, "hello from a fixture (edited)");
        assert!(message.edited_timestamp.is_some());
        // Fields the update did not carry are untouched.
        assert_eq!(message.author.username, "fake_friend");

        assert_eq!(
            store.apply(&event(fixture!("gateway/message_delete.json"))),
            Change::Messages(GENERAL)
        );
        assert!(store.messages(GENERAL).is_empty());
        // Deleting again changes nothing.
        assert_eq!(
            store.apply(&event(fixture!("gateway/message_delete.json"))),
            Change::None
        );
    }

    #[test]
    fn updates_for_unknown_messages_are_ignored() {
        let mut store = ready_store();
        assert_eq!(
            store.apply(&event(fixture!("gateway/message_update.json"))),
            Change::None
        );
        assert_eq!(
            store.apply(&event(fixture!("gateway/unknown_event.json"))),
            Change::None
        );
    }

    #[test]
    fn history_is_sorted_oldest_first_and_merged_without_duplicates() {
        let mut store = ready_store();
        // Live message arrives first, then history that also contains it.
        store.apply(&event(fixture!("gateway/message_create.json")));
        store.insert_history(GENERAL, history());
        store.insert_history(GENERAL, history());

        let ids: Vec<u64> = store.messages(GENERAL).iter().map(|m| m.id.get()).collect();
        assert_eq!(
            ids,
            [
                400_000_000_000_000_001,
                400_000_000_000_000_002,
                400_000_000_000_000_003
            ]
        );
    }

    #[test]
    fn history_for_another_channel_is_not_filed_under_this_one() {
        let mut store = ready_store();
        store.insert_history(Snowflake(1), history());
        assert!(store.messages(Snowflake(1)).is_empty());
        assert!(store.messages(GENERAL).is_empty());
    }

    #[test]
    fn per_channel_message_cap_drops_the_oldest() {
        let mut store = Store::new();
        let template = history().remove(0);
        for id in 1..=(MAX_MESSAGES_PER_CHANNEL as u64 + 10) {
            let mut message = template.clone();
            message.id = Snowflake(id);
            store.apply(&DispatchEvent::MessageCreate(Box::new(message)));
        }
        let messages = store.messages(GENERAL);
        assert_eq!(messages.len(), MAX_MESSAGES_PER_CHANNEL);
        assert_eq!(messages[0].id, Snowflake(11));
    }

    #[test]
    fn guild_create_adds_or_replaces_a_guild() {
        let mut store = ready_store();
        let guild: Guild = serde_json::from_str(
            r#"{"id":"200000000000000002","name":"Back Online",
                "channels":[{"id":"300000000000000900","type":0,"name":"lobby"}]}"#,
        )
        .unwrap();
        assert_eq!(
            store.apply(&DispatchEvent::GuildCreate(Box::new(guild))),
            Change::Guilds
        );
        assert_eq!(store.guilds().len(), 2);
        assert_eq!(store.guilds()[1].name, "Back Online");
        assert!(!store.guilds()[1].unavailable);
        assert!(store.channel(Snowflake(300_000_000_000_000_900)).is_some());
    }
}

pub mod cache;

#[cfg(test)]
mod event_tests {
    use super::*;
    use serde_json::json;
    fn unknown(name: &str, data: serde_json::Value) -> DispatchEvent {
        DispatchEvent::Unknown {
            name: name.into(),
            data,
        }
    }
    #[test]
    fn partial_channel_updates_preserve_other_fields_and_delete_removes_it() {
        let mut store = Store::new();
        store.apply(&unknown(
            "CHANNEL_CREATE",
            json!({"id":"1","type":1,"recipients":[{"id":"2","username":"friend"}]}),
        ));
        store.apply(&unknown(
            "CHANNEL_UPDATE",
            json!({"id":"1","name":"Renamed"}),
        ));
        assert_eq!(
            store.channel(Snowflake(1)).unwrap().kind,
            dc_protocol::model::ChannelType::Dm
        );
        assert_eq!(store.channel(Snowflake(1)).unwrap().recipients.len(), 1);
        store.apply(&unknown("CHANNEL_DELETE", json!({"id":"1"})));
        assert!(store.channel(Snowflake(1)).is_none());
        assert!(store.private_channels().is_empty());
    }
}
