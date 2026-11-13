//! The filesystem adapter for local, explicitly enabled plugin packages.
use dc_core::{
    plugin::{PluginInfo, PluginManifest},
    preferences::Preferences,
};
use dc_plugin_host::ScriptPlugin;
#[derive(Default)]
pub struct Plugins {
    scripts: Vec<ScriptPlugin>,
    pub info: Vec<PluginInfo>,
}
impl Plugins {
    pub fn load(preferences: &Preferences) -> Self {
        let mut plugins = Self::default();
        let Some(base) = crate::config::cosmetics_path()
            .map(|p| p.with_file_name("plugins"))
            .and_then(|p| p.canonicalize().ok())
        else {
            return plugins;
        };
        let Ok(entries) = std::fs::read_dir(&base) else {
            return plugins;
        };
        for entry in entries.flatten().take(16) {
            let Ok(dir) = entry.path().canonicalize() else {
                continue;
            };
            if !dir.starts_with(&base) || !dir.is_dir() {
                continue;
            }
            let Ok(path) = dir.join("plugin.json").canonicalize() else {
                continue;
            };
            if !path.starts_with(&base) {
                continue;
            }
            if std::fs::metadata(&path)
                .map(|m| m.len() > 16 * 1024)
                .unwrap_or(true)
            {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let Ok(manifest) = serde_json::from_str::<PluginManifest>(&text) else {
                continue;
            };
            let mut info = PluginInfo {
                manifest: manifest.clone(),
                enabled: false,
                error: None,
            };
            if let Err(error) = manifest.validate() {
                info.error = Some(error.into());
                plugins.info.push(info);
                continue;
            }
            if let Some(grant) = preferences.plugin_permissions.get(&manifest.id) {
                let source = dir
                    .join("plugin.js")
                    .canonicalize()
                    .ok()
                    .filter(|p| p.starts_with(&base))
                    .and_then(|p| {
                        if std::fs::metadata(&p)
                            .map(|m| m.len() > 256 * 1024)
                            .unwrap_or(true)
                        {
                            return None;
                        }
                        std::fs::read_to_string(p).ok()
                    });
                match source.and_then(|source| ScriptPlugin::load(manifest, &source, grant).ok()) {
                    Some(plugin) => {
                        info.enabled = true;
                        plugins.scripts.push(plugin);
                    }
                    None => {
                        info.error = Some(
                            "Plugin could not load, exceeded its limits, or lacks a grant".into(),
                        )
                    }
                }
            }
            plugins.info.push(info);
        }
        plugins
    }
    pub fn transform(&self, content: &str) -> Result<String, String> {
        let mut content = content.to_owned();
        for script in &self.scripts {
            content = script.transform(&content).map_err(|_| {
                format!(
                    "Plugin {} failed; your draft was kept",
                    script.manifest.name
                )
            })?;
        }
        Ok(content)
    }
}
