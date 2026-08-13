use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkspaceManifest {
    pub workspace: WorkspaceDetails,
    // BTreeMap so serialization order is deterministic — users may keep the
    // manifest (.myspace/config.toml) under version control, and a HashMap
    // would reorder entries on every save.
    #[serde(default)]
    pub repositories: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkspaceDetails {
    pub name: String,
    pub description: Option<String>,
}

impl WorkspaceManifest {
    pub fn load(manifest_path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(manifest_path)?;
        let manifest: WorkspaceManifest = toml::from_str(&content)?;
        Ok(manifest)
    }

    pub fn save(&self, manifest_path: &std::path::Path) -> anyhow::Result<()> {
        let content = toml::to_string(self)?;
        std::fs::write(manifest_path, content)?;
        Ok(())
    }
}
