//! The desktop UI.
//!
//! - [`model`]: pure view-models computed from the store and the overlay.
//!   All the "what to show" logic lives there and is unit-tested.
//! - [`view`]: the GPUI window that draws those view-models.
//!
//! The UI never talks to Discord itself. It receives [`BackendEvent`]s and
//! emits [`UiCommand`]s; the binary wires those to the gateway and REST
//! clients. It never sees the token.

pub mod model;
pub mod view;

use dc_protocol::Snowflake;
use dc_protocol::gateway::DispatchEvent;
use dc_protocol::model::Message;

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
    /// A one-line connection status for the status bar.
    Status(String),
}

/// What the UI asks the backend to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiCommand {
    /// Fetch recent messages for a channel the user opened.
    LoadHistory(Snowflake),
}

pub use view::run;
