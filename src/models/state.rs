use serde::{Deserialize, Serialize};
use std::path::Path;

/// Per-space, per-machine state (`<space>/.myspace/state.toml`). Kept apart
/// from the manifest deliberately: which branch *this machine's* IDE is
/// viewing is not part of the shareable space definition, and the file is
/// gitignored by the managed block.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SpaceState {
    /// Current canonical view: None = trunk, Some(name) = that branch set.
    pub view: Option<String>,
}

impl SpaceState {
    pub fn load(state_path: &Path) -> anyhow::Result<Self> {
        if !state_path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(state_path)?;
        Ok(toml::from_str(&content)?)
    }

    pub fn save(&self, state_path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = state_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(state_path, toml::to_string(self)?)?;
        Ok(())
    }
}
