use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

fn default_host() -> String {
    "github.com".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GlobalConfig {
    /// Host filled in when a target omits it. GitHub-shaped hosts only
    /// (host/org/repo, SSH transport) — includes GitHub Enterprise.
    #[serde(default = "default_host")]
    pub default_host: String,
    /// Org filled in when a target is a bare repo name.
    pub default_org: String,
    /// Aliases map a dot-less shorthand to a bare hostname,
    /// e.g. bb = "bitbucket.org". Identity only — never a transport string.
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    /// Per-host settings, keyed by hostname.
    #[serde(default)]
    pub hosts: HashMap<String, HostConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct HostConfig {
    /// Explicit SSH private key for this host (the IdentityFile+IdentitiesOnly
    /// equivalent). libgit2 does not read ~/.ssh/config, so agent key order
    /// decides identity by default — set this when multiple keys for the same
    /// host live in the agent. Supports a leading `~/`.
    pub ssh_key: Option<String>,
}

impl GlobalConfig {
    pub fn load(config_path: &std::path::Path) -> anyhow::Result<Self> {
        // A missing config is an error, not a default: falling back to a
        // placeholder default_org would silently resolve targets into
        // nonsense URLs.
        if !config_path.exists() {
            anyhow::bail!(
                "No config found at {:?}.\n\nCreate it with, for example:\n\n\
                 default_org = \"your-github-org-or-username\"\n\
                 # default_host = \"github.com\"   (optional; this is the default)\n\n\
                 # [aliases]\n\
                 # bb = \"bitbucket.org\"",
                config_path
            );
        }
        let content = std::fs::read_to_string(config_path)?;
        let config: GlobalConfig = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Invalid config at {:?}: {}", config_path, e))?;
        Ok(config)
    }

    /// The configured SSH identity for a host, tilde-expanded, if any.
    pub fn ssh_key_for(&self, host: &str) -> Option<PathBuf> {
        let raw = self.hosts.get(host)?.ssh_key.as_deref()?;
        Some(expand_tilde(raw))
    }
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}
