//! Loads the local cosmetics config from disk.

use std::path::PathBuf;

use anyhow::{Context, Result};
use dc_core::LocalOverlay;

/// `$DC_COSMETICS`, else `$XDG_CONFIG_HOME/dc-client/cosmetics.json`, else
/// `~/.config/dc-client/cosmetics.json` (`%APPDATA%` on Windows).
pub fn cosmetics_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DC_COSMETICS").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    base.map(|dir| dir.join("dc-client").join("cosmetics.json"))
}

/// A missing file means "no cosmetics"; a broken file is an error, so a typo
/// does not silently disable everything.
pub fn load_overlay() -> Result<LocalOverlay> {
    let Some(path) = cosmetics_path() else {
        return Ok(LocalOverlay::default());
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!(path = %path.display(), "no cosmetics config; using none");
            return Ok(LocalOverlay::default());
        }
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    LocalOverlay::from_json(&text).with_context(|| format!("parsing {}", path.display()))
}
