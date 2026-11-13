//! CDN URL builders (userdoccers "CDN Formatting"). Static PNGs only.

use crate::Snowflake;

pub const CDN_BASE: &str = "https://cdn.discordapp.com";

/// Discord's epoch (2015-01-01T00:00:00Z) in Unix milliseconds.
pub const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

/// Unix time in milliseconds at which a snowflake was created.
pub const fn snowflake_unix_ms(id: Snowflake) -> u64 {
    (id.0 >> 22) + DISCORD_EPOCH_MS
}

/// A user's avatar, or the default avatar when they have none.
pub fn avatar_url(user_id: Snowflake, avatar: Option<&str>, size: u16) -> String {
    match avatar.filter(|hash| !hash.is_empty()) {
        Some(hash) => format!("{CDN_BASE}/avatars/{user_id}/{hash}.png?size={size}"),
        // Docs: for migrated users the index is `(user_id >> 22) % 6`.
        None => format!("{CDN_BASE}/embed/avatars/{}.png", (user_id.0 >> 22) % 6),
    }
}

pub fn guild_icon_url(guild_id: Snowflake, icon: &str, size: u16) -> String {
    format!("{CDN_BASE}/icons/{guild_id}/{icon}.png?size={size}")
}

pub fn banner_url(user_id: Snowflake, banner: &str, size: u16) -> String {
    format!("{CDN_BASE}/banners/{user_id}/{banner}.png?size={size}")
}

/// An avatar decoration from Discord's catalog, by its asset hash.
pub fn avatar_decoration_url(asset: &str, size: u16) -> String {
    format!("{CDN_BASE}/avatar-decoration-presets/{asset}.png?size={size}&passthrough=false")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        let id = Snowflake(100_000_000_000_000_002);
        assert_eq!(
            avatar_url(id, Some("abc"), 64),
            "https://cdn.discordapp.com/avatars/100000000000000002/abc.png?size=64"
        );
        assert_eq!(
            avatar_url(id, None, 64),
            format!("https://cdn.discordapp.com/embed/avatars/{}.png", (id.0 >> 22) % 6)
        );
        assert_eq!(avatar_url(id, Some(""), 64), avatar_url(id, None, 64));
        assert_eq!(
            guild_icon_url(Snowflake(5), "i", 96),
            "https://cdn.discordapp.com/icons/5/i.png?size=96"
        );
        assert_eq!(
            avatar_decoration_url("a_1", 96),
            "https://cdn.discordapp.com/avatar-decoration-presets/a_1.png?size=96&passthrough=false"
        );
        assert_eq!(snowflake_unix_ms(Snowflake(0)), DISCORD_EPOCH_MS);
    }
}
