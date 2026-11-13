//! First-party emoji insertion and FakeNitro-lite URLs. No account spoofing.
use dc_protocol::{Snowflake, members::CustomEmoji};

pub const UNICODE: &[(&str, &str)] = &[
    ("😀", "grinning smile happy"),
    ("😄", "smile happy"),
    ("😁", "grin"),
    ("😂", "laugh tears joy"),
    ("🤣", "rofl laugh"),
    ("😊", "blush smile"),
    ("🥰", "love smile"),
    ("😍", "heart eyes"),
    ("😎", "cool sunglasses"),
    ("🥹", "pleading happy"),
    ("😭", "cry sob"),
    ("😅", "sweat smile"),
    ("🤔", "thinking"),
    ("🙃", "upside down"),
    ("🫠", "melting"),
    ("👀", "eyes"),
    ("✨", "sparkles"),
    ("💜", "purple heart"),
    ("❤️", "red heart love"),
    ("💕", "hearts"),
    ("👍", "thumbs up"),
    ("👎", "thumbs down"),
    ("👋", "wave hello"),
    ("🙌", "celebrate"),
    ("🙏", "thanks pray"),
    ("🎉", "party tada"),
    ("🔥", "fire"),
    ("💯", "hundred"),
    ("✅", "check done"),
    ("❌", "cross no"),
    ("🐱", "cat"),
    ("🐶", "dog"),
    ("🦀", "crab rust"),
    ("🌸", "cherry blossom"),
    ("🌙", "moon night"),
    ("☀️", "sun"),
    ("⭐", "star"),
    ("🌈", "rainbow"),
    ("☕", "coffee"),
    ("🍕", "pizza"),
];
pub fn url(id: Snowflake, animated: bool, size: u16) -> String {
    format!(
        "https://cdn.discordapp.com/emojis/{id}.{}?size={}&quality=lossless",
        if animated { "gif" } else { "png" },
        size.clamp(16, 256).next_power_of_two()
    )
}
pub fn insertion(emoji: &CustomEmoji, as_link: bool, size: u16) -> String {
    if as_link {
        url(emoji.id, emoji.animated, size)
    } else {
        format!(
            "<{}:{}:{}>",
            if emoji.animated { "a" } else { "" },
            emoji.name,
            emoji.id
        )
    }
}
/// Only standalone Discord emoji URLs are candidates for inline image rendering.
pub fn image_links(content: &str) -> Vec<String> {
    content
        .split_whitespace()
        .filter_map(|word| {
            let value = word.trim_matches(['<', '>']);
            let tail = value.strip_prefix("https://cdn.discordapp.com/emojis/")?;
            let (id, format) = tail.split_once('.')?;
            id.parse::<Snowflake>().ok()?;
            let format = format.split('?').next()?;
            matches!(format, "png" | "gif" | "webp").then(|| value.to_owned())
        })
        .take(20)
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_discord_emoji_assets_become_images() {
        let image = url(Snowflake(42), true, 128);
        assert_eq!(image_links(&format!("hi {image}")), [image]);
        assert!(
            image_links(
                "https://example.invalid/emojis/42.png https://cdn.discordapp.com/emojis/42.svg"
            )
            .is_empty()
        );
        assert_eq!(
            url(Snowflake(1), false, 17),
            "https://cdn.discordapp.com/emojis/1.png?size=32&quality=lossless"
        );
    }
}
