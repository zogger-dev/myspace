use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GlobalConfig {
    pub default_org: String,
    pub default_scm: String,
    #[serde(default)]
    pub aliases: HashMap<String, String>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            default_org: "default_org".to_string(),
            default_scm: "git@github.com".to_string(),
            aliases: HashMap::new(),
        }
    }
}

impl GlobalConfig {
    pub fn load(config_path: &std::path::Path) -> anyhow::Result<Self> {
        if config_path.exists() {
            let content = std::fs::read_to_string(config_path)?;
            let config: GlobalConfig = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }
}
