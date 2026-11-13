//! Versioned, capability-based first-party and JavaScript extension boundary.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub api_version: u32,
    #[serde(default)]
    pub capabilities: Vec<String>,
}
impl PluginManifest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.api_version != 1 {
            return Err("Unsupported plugin API version");
        }
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("Invalid plugin ID");
        }
        if self.name.is_empty() || self.name.len() > 128 {
            return Err("Invalid plugin name");
        }
        if self.capabilities.iter().any(|c| c != "outgoing-transform") {
            return Err(
                "This build only supports outgoing-transform; no IO capabilities are granted",
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginInfo {
    pub manifest: PluginManifest,
    pub enabled: bool,
    pub error: Option<String>,
}

/// Tier A plugins use the same pure transform boundary. They never receive
/// credentials, transports or handles to mutable server state.
pub trait OutgoingTransform {
    fn transform(&self, content: &str) -> Result<String, String>;
}
