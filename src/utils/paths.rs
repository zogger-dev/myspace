use anyhow::{Result, anyhow};
use std::env;
use std::fs;
use std::path::PathBuf;

/// Root for all machine-global myspace state: `$MYSPACE_HOME` if set,
/// otherwise `~/.myspace`. A flat, predictable location on every platform
/// (cargo/rustup convention) rather than the OS application-support dirs.
pub fn myspace_home() -> Result<PathBuf> {
    if let Ok(home) = env::var("MYSPACE_HOME")
        && !home.is_empty()
    {
        return Ok(PathBuf::from(home));
    }
    dirs::home_dir()
        .map(|home| home.join(".myspace"))
        .ok_or_else(|| anyhow!("Could not determine the home directory"))
}

/// Path of the global config file. Not created on demand — a missing config
/// is a meaningful state (see GlobalConfig::load).
pub fn get_config_path() -> Result<PathBuf> {
    Ok(myspace_home()?.join("config.toml"))
}

/// Directory holding the shared bare-repo cache, laid out as
/// `<host>/<org>/<repo>`.
pub fn get_bare_repos_dir() -> Result<PathBuf> {
    let path = myspace_home()?.join("repos");
    if !path.exists() {
        fs::create_dir_all(&path)?;
    }
    Ok(path)
}

/// Directory for rotating log files.
pub fn get_logs_dir() -> Result<PathBuf> {
    let path = myspace_home()?.join("logs");
    if !path.exists() {
        fs::create_dir_all(&path)?;
    }
    Ok(path)
}
