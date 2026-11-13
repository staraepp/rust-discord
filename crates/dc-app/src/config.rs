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

/// Saves through a same-directory temporary file, then atomically replaces
/// the destination. A failed write leaves the previous configuration intact.
pub fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = parent.join(format!(".dc-write-{}-{stamp}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<()> {
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
pub fn save_overlay(json: &str) -> Result<()> {
    let overlay = LocalOverlay::from_json(json)?;
    let path = cosmetics_path().context("No configuration directory is available")?;
    atomic_write(&path, overlay.to_json().as_bytes())
}
pub fn preferences_path() -> Option<PathBuf> {
    cosmetics_path().map(|p| p.with_file_name("settings.json"))
}
pub fn load_preferences() -> Result<dc_core::preferences::Preferences> {
    let Some(path) = preferences_path() else {
        return Ok(Default::default());
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(e) => return Err(e.into()),
    };
    let prefs: dc_core::preferences::Preferences = serde_json::from_str(&text)?;
    prefs.validate().map_err(anyhow::Error::msg)?;
    Ok(prefs)
}
pub fn save_preferences(prefs: &dc_core::preferences::Preferences) -> Result<()> {
    prefs.validate().map_err(anyhow::Error::msg)?;
    let path = preferences_path().context("No configuration directory is available")?;
    atomic_write(&path, &serde_json::to_vec_pretty(prefs)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_replace_and_failed_rename_leave_no_partial_file() {
        let root = std::env::temp_dir().join(format!(
            "dc-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("cosmetics.json");
        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let directory = root.join("occupied");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("keep"), b"preserve").unwrap();
        assert!(atomic_write(&directory, b"bad").is_err());
        assert_eq!(std::fs::read(directory.join("keep")).unwrap(), b"preserve");
        assert!(std::fs::read_dir(&root).unwrap().all(|e| {
            !e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".dc-write-")
        }));
        std::fs::remove_dir_all(root).unwrap();
    }
}
