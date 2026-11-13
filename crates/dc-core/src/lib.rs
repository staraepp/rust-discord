//! Client core. For now: the [`overlay`] (local-only cosmetics).
//!
//! The event bus and plugin traits from blueprint §4 are not here yet.

pub mod overlay;

pub use overlay::{LocalOverlay, OverlayError, ProfileOverride, UserView};
