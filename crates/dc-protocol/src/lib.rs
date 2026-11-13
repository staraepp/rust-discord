//! Pure wire types for the Discord gateway and REST API (user accounts).
//!
//! No IO lives here. Source of truth for every payload is the unofficial
//! user-API documentation listed in `ref.md`; anything those docs do not pin
//! down is marked `// ASSUMPTION:` and listed in `docs/PROGRESS.md`.
//!
//! Parsing is deliberately tolerant: unknown fields are ignored (or kept in an
//! `extra` map on the small, frequently-inspected objects) and most fields
//! have defaults, because the user API changes without notice.

pub mod cdn;
pub mod close;
pub mod gateway;
pub mod model;
pub mod secret;
pub mod snowflake;
pub mod time;

pub use secret::Secret;
pub use snowflake::Snowflake;

/// Gateway/REST API version this crate models.
pub const API_VERSION: u8 = 9;

/// Errors produced while decoding wire payloads.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// The frame was not a valid gateway payload envelope.
    #[error("malformed gateway payload: {0}")]
    MalformedPayload(#[source] serde_json::Error),
    /// A known event's `d` did not match the expected model.
    #[error("malformed `{event}` event data: {source}")]
    MalformedEvent {
        event: String,
        #[source]
        source: serde_json::Error,
    },
}

pub mod members;
