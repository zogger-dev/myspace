use serde::{Deserialize, Serialize};

use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkspaceManifest {
    pub workspace: WorkspaceDetails,
    #[serde(default)]
    pub repositories: HashMap<String, String>,
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
