use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// A best-effort list of known space roots (`~/.myspace/spaces.toml`), kept
/// so `sync --all`/`status --all` can find spaces without scanning the disk.
/// It is advisory, never authoritative: entries are verified against the
/// on-disk `.myspace/config.toml` marker on read and pruned when stale, so
/// hand-moved or hand-deleted spaces cannot poison anything.
#[derive(Debug, Serialize, Deserialize, Default)]
struct Registry {
    #[serde(default)]
    spaces: Vec<PathBuf>,
}

fn load(registry_path: &Path) -> Registry {
    fs::read_to_string(registry_path)
        .ok()
        .and_then(|content| toml::from_str(&content).ok())
        .unwrap_or_default()
}

fn save(registry_path: &Path, registry: &Registry) -> Result<()> {
    if let Some(parent) = registry_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(registry_path, toml::to_string(registry)?)?;
    Ok(())
}

pub fn register_space(registry_path: &Path, space_root: &Path) -> Result<()> {
    let space_root = space_root
        .canonicalize()
        .unwrap_or_else(|_| space_root.to_path_buf());
    let mut registry = load(registry_path);
    if !registry.spaces.contains(&space_root) {
        registry.spaces.push(space_root);
        registry.spaces.sort();
        save(registry_path, &registry)?;
    }
    Ok(())
}

pub fn unregister_space(registry_path: &Path, space_root: &Path) -> Result<()> {
    let space_root = space_root
        .canonicalize()
        .unwrap_or_else(|_| space_root.to_path_buf());
    let mut registry = load(registry_path);
    registry.spaces.retain(|p| p != &space_root);
    save(registry_path, &registry)?;
    Ok(())
}

/// Registered spaces that still exist on disk. Stale entries are pruned
/// from the file as a side effect.
pub fn registered_spaces(registry_path: &Path) -> Result<Vec<PathBuf>> {
    let mut registry = load(registry_path);
    let before = registry.spaces.len();
    registry
        .spaces
        .retain(|p| p.join(".myspace").join("config.toml").exists());
    if registry.spaces.len() != before {
        save(registry_path, &registry)?;
    }
    Ok(registry.spaces)
}
