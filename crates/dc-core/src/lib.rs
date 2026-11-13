//! Client core. For now: the [`overlay`] (local-only cosmetics).
//!
//! The event bus and plugin traits from blueprint §4 are not here yet.

pub mod composer;
pub mod overlay;

pub use overlay::{LocalOverlay, OverlayError, ProfileOverride, UserView};

pub mod markdown;

pub mod emoji;

pub mod preferences;

pub mod plugin;
