use anyhow::{Result, anyhow};
use directories::ProjectDirs;
use std::fs;
use std::path::PathBuf;

fn get_project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("com", "myspace", "myspace")
        .ok_or_else(|| anyhow!("Could not determine project directories"))
}

pub fn get_config_dir() -> Result<PathBuf> {
    let dirs = get_project_dirs()?;
    let path = dirs.config_dir().to_path_buf();
    if !path.exists() {
        fs::create_dir_all(&path)?;
    }
    Ok(path)
}

pub fn get_data_dir() -> Result<PathBuf> {
    let dirs = get_project_dirs()?;
    let path = dirs.data_dir().to_path_buf();
    if !path.exists() {
        fs::create_dir_all(&path)?;
    }
    Ok(path)
}

pub fn get_state_dir() -> Result<PathBuf> {
    let dirs = get_project_dirs()?;
    let path = dirs
        .state_dir()
        .unwrap_or_else(|| dirs.data_dir())
        .to_path_buf();
    if !path.exists() {
        fs::create_dir_all(&path)?;
    }
    Ok(path)
}

pub fn get_bare_repos_dir() -> Result<PathBuf> {
    let mut data_dir = get_data_dir()?;
    data_dir.push("repos");
    if !data_dir.exists() {
        fs::create_dir_all(&data_dir)?;
    }
    Ok(data_dir)
}
