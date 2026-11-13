//! The desktop UI.
//!
//! - [`model`]: pure view-models computed from the store and the overlay.
//!   All the "what to show" logic lives there and is unit-tested.
//! - [`view`]: the GPUI window that draws those view-models.
//!
//! The UI never talks to Discord itself. It receives [`BackendEvent`]s and
//! emits [`UiCommand`]s; the binary wires those to the gateway and REST
//! clients. It never sees the token.

pub mod input;
pub mod model;
pub mod view;

use dc_protocol::Snowflake;
use dc_protocol::gateway::DispatchEvent;
use dc_protocol::model::{CreateMessage, Message};

/// What the backend tells the UI.
#[derive(Debug)]
pub enum BackendEvent {
    /// A gateway dispatch to fold into the store.
    Dispatch(DispatchEvent),
    /// A page of REST history for a channel.
    History {
        channel_id: Snowflake,
        messages: Vec<Message>,
    },
    OlderHistory {
        channel_id: Snowflake,
        messages: Vec<Message>,
    },
    MessageChanged(Box<Message>),
    MessageRemoved {
        channel_id: Snowflake,
        message_id: Snowflake,
    },
    OperationFailed(String),
    Plugins(Vec<dc_core::plugin::PluginInfo>),
    OverlaySaved {
        json: String,
        error: Option<String>,
    },
    PreferencesSaved {
        error: Option<String>,
    },
    Sent {
        channel_id: Snowflake,
        nonce: String,
        message: Box<Message>,
    },
    SendFailed {
        channel_id: Snowflake,
        nonce: String,
        error: String,
    },
    /// A one-line connection status for the status bar.
    Status(String),
    /// A CDN image the UI asked for, as raw bytes.
    Image {
        url: String,
        bytes: Vec<u8>,
    },
    /// A CDN image that could not be downloaded; the UI keeps its fallback.
    ImageFailed {
        url: String,
    },
}

/// What the UI asks the backend to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiCommand {
    /// Fetch recent messages for a channel the user opened.
    LoadHistory(Snowflake),
    SubscribeMembers {
        guild_id: Snowflake,
        channel_id: Snowflake,
        offset: usize,
    },
    LoadOlder {
        channel_id: Snowflake,
        before: Snowflake,
    },
    EditMessage {
        channel_id: Snowflake,
        message_id: Snowflake,
        content: String,
    },
    DeleteMessage {
        channel_id: Snowflake,
        message_id: Snowflake,
    },
    ClearHistoryCache,
    SaveOverlay(String),
    SavePreferences(dc_core::preferences::Preferences),
    SendMessage {
        channel_id: Snowflake,
        message: CreateMessage,
        attachments: Vec<std::path::PathBuf>,
    },
    /// Download a Discord CDN image (avatar, icon, banner, decoration).
    FetchImage(String),
}

pub use view::{UiOptions, run};
