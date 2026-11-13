//! Local application preferences. Never stores credentials.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub fake_nitro: bool,
    pub emoji_size: u16,
    pub font_size: u8,
    pub cache_history: bool,
    pub risk_accepted: bool,
    pub plugin_permissions: std::collections::HashMap<String, Vec<String>>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            fake_nitro: true,
            emoji_size: 128,
            font_size: 14,
            cache_history: false,
            risk_accepted: false,
            plugin_permissions: Default::default(),
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(16..=256).contains(&self.emoji_size) {
            return Err("Emoji size must be 16–256");
        }
        if !(11..=24).contains(&self.font_size) {
            return Err("Font size must be 11–24");
        }
        Ok(())
    }
}
