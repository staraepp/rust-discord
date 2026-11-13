//! Collect packaged license/notice texts for development binary artifacts.
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn notices(root: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<()> {
    if depth > 5 {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().to_uppercase();
        if kind.is_dir() && !matches!(name.as_str(), "TARGET" | ".GIT") {
            notices(&entry.path(), depth + 1, out)?;
        } else if kind.is_file()
            && ["LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
            && entry.metadata()?.len() <= 1024 * 1024
        {
            out.push(entry.path());
        }
    }
    Ok(())
}
pub fn collect(destination: &Path) -> Result<()> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let compiler = Command::new("rustc").arg("-vV").output()?;
    let version = String::from_utf8(compiler.stdout)?;
    let host = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("rustc did not report a host target")?;
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--offline",
            "--filter-platform",
            host,
        ])
        .output()?;
    if !output.status.success() {
        return Err("cargo metadata failed while collecting licenses".into());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let workspace: HashSet<&str> = metadata["workspace_members"]
        .as_array()
        .ok_or("missing workspace members")?
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    let mut packages = metadata["packages"]
        .as_array()
        .ok_or("missing packages")?
        .iter()
        .filter(|p| !workspace.contains(p["id"].as_str().unwrap_or("")))
        .collect::<Vec<_>>();
    packages.sort_by_key(|p| {
        (
            p["name"].as_str().unwrap_or(""),
            p["version"].as_str().unwrap_or(""),
        )
    });
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(destination)?;
    writeln!(
        file,
        "Third-party package license and copyright notices\nResolved versions are pinned in Cargo.lock. Project licenses are distributed separately.\n"
    )?;
    for package in &packages {
        let manifest = Path::new(
            package["manifest_path"]
                .as_str()
                .ok_or("missing manifest path")?,
        );
        let root = manifest.parent().ok_or("missing package root")?;
        writeln!(
            file,
            "\n=== {} {} ===\nDeclared license: {}\nUpstream: {}\n",
            package["name"].as_str().unwrap_or(""),
            package["version"].as_str().unwrap_or(""),
            package["license"]
                .as_str()
                .unwrap_or("see included license files"),
            package["repository"]
                .as_str()
                .unwrap_or("see Cargo.lock source")
        )?;
        let mut paths = Vec::new();
        notices(root, 0, &mut paths)?;
        if let Some(path) = package["license_file"].as_str() {
            let path = Path::new(path);
            let path = if path.is_absolute() {
                path.to_owned()
            } else {
                root.join(path)
            };
            if let Ok(path) = path.canonicalize()
                && path.starts_with(root.canonicalize()?)
                && path.is_file()
            {
                paths.push(path);
            }
        }
        paths.sort();
        paths.dedup();
        for path in paths {
            if let Ok(text) = std::fs::read_to_string(&path) {
                writeln!(
                    file,
                    "--- {} ---\n{text}\n",
                    path.strip_prefix(root).unwrap_or(&path).display()
                )?;
            }
        }
    }
    println!(
        "Collected notices for {} packages into {}",
        packages.len(),
        destination.display()
    );
    Ok(())
}
