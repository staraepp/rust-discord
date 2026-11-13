//! `LocalOverlay`: cosmetics that exist only on this machine.
//!
//! The overlay sits between the store and the renderer (blueprint §5). It
//! never changes server data and never sends anything to Discord: it turns a
//! `User` into a [`UserView`] with locally configured extras layered on top.
//! Nobody else sees any of it.
//!
//! The configuration is a JSON document:
//!
//! ```json
//! {
//!   "me": { "accent_color": "#ff66aa", "banner": "/home/me/banner.png" },
//!   "users": {
//!     "100000000000000002": { "display_name": "bestie", "name_color": "#00ccff" }
//!   }
//! }
//! ```

use std::collections::HashMap;

use dc_protocol::Snowflake;
use dc_protocol::model::User;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum OverlayError {
    #[error("invalid cosmetics config: {0}")]
    Invalid(#[from] serde_json::Error),
}

/// An opaque RGB color, written as `"#rrggbb"` (or `"#rgb"`) in the config.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color(pub u32);

impl Color {
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        match hex.len() {
            6 => u32::from_str_radix(hex, 16).ok().map(Color),
            3 => {
                let v = u32::from_str_radix(hex, 16).ok()?;
                let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
                Some(Color((r * 17) << 16 | (g * 17) << 8 | (b * 17)))
            }
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Color::parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format!("`{text}` is not a color like \"#ff66aa\""))
        })
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("#{:06x}", self.0 & 0x00ff_ffff))
    }
}

/// Local cosmetic overrides for one user. Every field is optional; an absent
/// field leaves what Discord sent untouched.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileOverride {
    /// Shown instead of the user's display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Color of the name wherever it is rendered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_color: Option<Color>,
    /// Profile accent ("banner") color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_color: Option<Color>,
    /// Profile banner image: a local file path or an https URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner: Option<String>,
    /// Avatar image: a local file path or an https URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// Avatar decoration image: a local file path or an https URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_decoration: Option<String>,
}

/// The whole cosmetics configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalOverlay {
    /// Overrides for the logged-in account, whoever that is.
    #[serde(default)]
    pub me: ProfileOverride,
    /// Overrides for specific users, by user ID.
    #[serde(default)]
    pub users: HashMap<Snowflake, ProfileOverride>,
}

/// A user as the UI should draw them: server data with local overrides
/// applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserView {
    pub id: Snowflake,
    pub name: String,
    pub name_color: Option<Color>,
    pub accent_color: Option<Color>,
    pub banner: Option<String>,
    /// Local override only; Discord's own avatar is not resolved here yet.
    pub avatar: Option<String>,
    pub avatar_decoration: Option<String>,
    /// True if any local override changed how this user looks.
    pub customized: bool,
}

impl LocalOverlay {
    /// Parses the JSON config. Unknown keys are an error, so a typo such as
    /// `"acent_color"` is reported instead of silently doing nothing.
    pub fn from_json(text: &str) -> Result<Self, OverlayError> {
        Ok(serde_json::from_str(text)?)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_owned())
    }

    /// Applies the overlay to a user. `current_user` is the logged-in
    /// account's ID, if known; the `me` block applies to that user, and a
    /// `users` entry for the same ID wins field by field.
    pub fn view_user(&self, user: &User, current_user: Option<Snowflake>) -> UserView {
        let me = (current_user == Some(user.id)).then_some(&self.me);
        let specific = self.users.get(&user.id);

        fn pick<T: Clone>(
            specific: Option<&ProfileOverride>,
            me: Option<&ProfileOverride>,
            field: impl Fn(&ProfileOverride) -> &Option<T>,
        ) -> Option<T> {
            specific
                .and_then(|o| field(o).clone())
                .or_else(|| me.and_then(|o| field(o).clone()))
        }

        let display_name = pick(specific, me, |o| &o.display_name).filter(|n| !n.is_empty());
        let view = UserView {
            id: user.id,
            name: display_name
                .clone()
                .unwrap_or_else(|| user.display_name().to_owned()),
            name_color: pick(specific, me, |o| &o.name_color),
            accent_color: pick(specific, me, |o| &o.accent_color),
            banner: pick(specific, me, |o| &o.banner),
            avatar: pick(specific, me, |o| &o.avatar),
            avatar_decoration: pick(specific, me, |o| &o.avatar_decoration),
            customized: false,
        };
        let customized = display_name.is_some()
            || view.name_color.is_some()
            || view.accent_color.is_some()
            || view.banner.is_some()
            || view.avatar.is_some()
            || view.avatar_decoration.is_some();
        UserView { customized, ..view }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: Snowflake = Snowflake(100_000_000_000_000_001);
    const FRIEND: Snowflake = Snowflake(100_000_000_000_000_002);

    fn user(id: Snowflake, username: &str, global: Option<&str>) -> User {
        User {
            id,
            username: username.to_owned(),
            discriminator: "0".to_owned(),
            global_name: global.map(str::to_owned),
            avatar: None,
            bot: false,
        }
    }

    fn overlay() -> LocalOverlay {
        LocalOverlay::from_json(
            r##"{
                "me": {
                    "accent_color": "#ff66aa",
                    "banner": "/home/fake/banner.png",
                    "avatar_decoration": "/home/fake/decor.png"
                },
                "users": {
                    "100000000000000002": {"display_name": "bestie", "name_color": "#0cf"},
                    "100000000000000001": {"accent_color": "#000001"}
                }
            }"##,
        )
        .unwrap()
    }

    #[test]
    fn colors_parse_and_round_trip() {
        assert_eq!(Color::parse("#ff66aa"), Some(Color(0xff66aa)));
        assert_eq!(Color::parse("FF66AA"), Some(Color(0xff66aa)));
        assert_eq!(Color::parse("#0cf"), Some(Color(0x00ccff)));
        assert_eq!(Color::parse("#12345"), None);
        assert_eq!(Color::parse("#gggggg"), None);
        assert_eq!(Color::parse("+f66aa"), None);
        assert_eq!(
            serde_json::to_string(&Color(0x00ccff)).unwrap(),
            "\"#00ccff\""
        );
    }

    #[test]
    fn empty_overlay_changes_nothing() {
        let view =
            LocalOverlay::default().view_user(&user(ME, "fake_owner", Some("Owner")), Some(ME));
        assert_eq!(view.name, "Owner");
        assert!(!view.customized);
        assert_eq!(view.accent_color, None);
        assert_eq!(view.banner, None);
    }

    #[test]
    fn me_block_applies_only_to_the_current_user() {
        let overlay = overlay();
        let mine = overlay.view_user(&user(ME, "fake_owner", None), Some(ME));
        assert_eq!(mine.banner.as_deref(), Some("/home/fake/banner.png"));
        assert_eq!(
            mine.avatar_decoration.as_deref(),
            Some("/home/fake/decor.png")
        );
        assert!(mine.customized);

        let stranger = overlay.view_user(&user(Snowflake(9), "someone", None), Some(ME));
        assert!(!stranger.customized);
        assert_eq!(stranger.banner, None);

        // Before login is known, `me` applies to nobody.
        let unknown = overlay.view_user(&user(ME, "fake_owner", None), None);
        assert_eq!(unknown.banner, None);
    }

    #[test]
    fn per_user_entry_wins_over_me_field_by_field() {
        let mine = overlay().view_user(&user(ME, "fake_owner", None), Some(ME));
        // accent_color comes from the users entry, banner still from `me`.
        assert_eq!(mine.accent_color, Some(Color(0x000001)));
        assert_eq!(mine.banner.as_deref(), Some("/home/fake/banner.png"));
    }

    #[test]
    fn other_users_can_be_customized() {
        let view = overlay().view_user(&user(FRIEND, "fake_friend", Some("Fake Friend")), Some(ME));
        assert_eq!(view.name, "bestie");
        assert_eq!(view.name_color, Some(Color(0x00ccff)));
        assert!(view.customized);
    }

    #[test]
    fn empty_display_name_falls_back_to_the_real_one() {
        let overlay = LocalOverlay::from_json(r#"{"me":{"display_name":""}}"#).unwrap();
        let view = overlay.view_user(&user(ME, "fake_owner", None), Some(ME));
        assert_eq!(view.name, "fake_owner");
        assert!(!view.customized);
    }

    #[test]
    fn typos_and_bad_values_are_errors() {
        assert!(LocalOverlay::from_json(r##"{"me":{"acent_color":"#fff"}}"##).is_err());
        assert!(LocalOverlay::from_json(r#"{"me":{"accent_color":"pink"}}"#).is_err());
        assert!(LocalOverlay::from_json(r#"{"users":{"not-an-id":{}}}"#).is_err());
        assert!(LocalOverlay::from_json("{}").is_ok());
    }

    #[test]
    fn config_round_trips() {
        let overlay = overlay();
        assert_eq!(
            LocalOverlay::from_json(&overlay.to_json()).unwrap(),
            overlay
        );
    }
}
