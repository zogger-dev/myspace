mod cli;
mod engine;
mod git;
mod models;
mod utils;

use anyhow::{Result, anyhow};
use clap::Parser;
use cli::commands::{Cli, Commands};
use indicatif::{ProgressBar, ProgressStyle};
use models::config::GlobalConfig;
use models::manifest::{WorkspaceDetails, WorkspaceManifest};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use tokio::process::Command;
use utils::context::find_workspace_root;

fn bare_repo_cache_key(git_url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(git_url.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn bare_repo_path(cache_dir: &Path, git_url: &str) -> PathBuf {
    cache_dir.join(bare_repo_cache_key(git_url))
}

fn validate_workspace_subpath(name: &str) -> Result<()> {
    let name_path = Path::new(name);
    if name_path.is_absolute()
        || name_path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(anyhow!(
            "Workspace name must be a relative path under the current directory"
        ));
    }
    Ok(())
}

fn resolve_named_workspace(name: &str) -> Result<PathBuf> {
    validate_workspace_subpath(name)?;
    let cwd = env::current_dir()?.canonicalize()?;
    let workspace_root = cwd.join(name);
    if !workspace_root.exists() {
        return Err(anyhow!("Workspace not found at {:?}", workspace_root));
    }
    let workspace_root = workspace_root.canonicalize()?;
    if !workspace_root.starts_with(&cwd) {
        return Err(anyhow!(
            "Workspace path must stay within the current directory"
        ));
    }

    Ok(workspace_root)
}

async fn handle_init() -> Result<()> {
    let cwd = env::current_dir()?;
    let manifest_path = cwd.join("myspace.toml");
    if manifest_path.exists() {
        return Err(anyhow!(
            "myspace.toml already exists in the current directory"
        ));
    }

    let manifest = WorkspaceManifest {
        workspace: WorkspaceDetails {
            name: cwd
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            description: Some("Auto-generated workspace".to_string()),
        },
        repositories: std::collections::HashMap::new(),
    };

    let content = toml::to_string(&manifest)?;
    fs::write(&manifest_path, content)?;
    println!("Initialized myspace workspace in {:?}", cwd);
    Ok(())
}

async fn handle_create(name: &str) -> Result<()> {
    validate_workspace_subpath(name)?;
    let mut cwd = env::current_dir()?;
    cwd.push(name);

    if cwd.exists() {
        return Err(anyhow!("Directory {} already exists", name));
    }

    fs::create_dir_all(&cwd)?;

    let manifest_path = cwd.join("myspace.toml");
    let manifest = WorkspaceManifest {
        workspace: WorkspaceDetails {
            name: name.to_string(),
            description: Some(format!("Workspace {}", name)),
        },
        repositories: std::collections::HashMap::new(),
    };

    let content = toml::to_string(&manifest)?;
    fs::write(&manifest_path, content)?;
    println!("Created workspace {} at {:?}", name, cwd);
    Ok(())
}

async fn handle_edit(name: Option<String>) -> Result<()> {
    let manifest_path = if let Some(n) = name {
        resolve_named_workspace(&n)?.join("myspace.toml")
    } else {
        find_workspace_root()
            .ok_or_else(|| anyhow!("Not in a myspace workspace"))?
            .join("myspace.toml")
    };

    if !manifest_path.exists() {
        return Err(anyhow!("Manifest not found at {:?}", manifest_path));
    }

    let editor = env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    let status = Command::new(&editor).arg(&manifest_path).status().await?;
    if !status.success() {
        return Err(anyhow!("Editor '{}' exited unsuccessfully", editor));
    }

    Ok(())
}

async fn handle_add(target: &str) -> Result<()> {
    let workspace_root =
        find_workspace_root().ok_or_else(|| anyhow!("Not in a myspace workspace"))?;

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );
    spinner.set_message(format!("Resolving target {}...", target));
    spinner.enable_steady_tick(std::time::Duration::from_millis(120));

    let config_dir = utils::paths::get_config_dir()?;
    let config_path = config_dir.join("config.toml");
    let global_config = GlobalConfig::load(&config_path)?;

    let parsed_target = engine::target_parser::parse_target(target)?;
    let git_url = engine::target_parser::resolve_git_url(&parsed_target, &global_config)?;

    spinner.set_message(format!("Cloning bare repository from {}...", git_url));

    // Determine bare repo path
    let cache_dir = utils::paths::get_bare_repos_dir()?;
    let bare_repo_path = bare_repo_path(&cache_dir, &git_url);

    git::engine::clone_bare(&git_url, &bare_repo_path, Some(&spinner)).await?;

    let worktree_name = match &parsed_target {
        models::target::Target::Bare(repo) => repo.clone(),
        models::target::Target::OrgRepo(_, repo) => repo.clone(),
        models::target::Target::Aliased(_, _, repo) => repo.clone(),
    };

    let worktree_path = workspace_root.join(&worktree_name);

    spinner.set_message(format!("Provisioning worktree {}...", worktree_name));
    git::engine::add_worktree(&bare_repo_path, &worktree_path, Some(&spinner)).await?;

    let manifest_path = workspace_root.join("myspace.toml");
    let mut manifest = WorkspaceManifest::load(&manifest_path)?;
    manifest.repositories.insert(worktree_name.clone(), git_url);
    manifest.save(&manifest_path)?;

    spinner.finish_with_message(format!("Successfully added {} to workspace", worktree_name));
    Ok(())
}

async fn handle_delete(name: Option<String>) -> Result<()> {
    let workspace_root = if let Some(n) = name {
        resolve_named_workspace(&n)?
    } else {
        find_workspace_root()
            .ok_or_else(|| anyhow!("Not in a myspace workspace"))?
            .canonicalize()?
    };

    if !workspace_root.exists() {
        return Err(anyhow!("Workspace not found at {:?}", workspace_root));
    }

    let manifest_path = workspace_root.join("myspace.toml");
    if !manifest_path.exists() {
        return Err(anyhow!("Manifest not found at {:?}", manifest_path));
    }
    let manifest = WorkspaceManifest::load(&manifest_path)?;

    let current_dir = env::current_dir()?.canonicalize()?;
    if current_dir.starts_with(&workspace_root) {
        let mut fallback_dirs = Vec::new();
        if let Some(home) = dirs::home_dir().and_then(|path| path.canonicalize().ok()) {
            fallback_dirs.push(home);
        }
        if let Some(parent) = workspace_root
            .parent()
            .and_then(|path| path.canonicalize().ok())
        {
            fallback_dirs.push(parent);
        }
        fallback_dirs.push(env::temp_dir());

        let fallback_dir = fallback_dirs
            .into_iter()
            .find(|path| !path.starts_with(&workspace_root))
            .ok_or_else(|| anyhow!("Cannot determine a safe directory outside the workspace"))?;
        env::set_current_dir(&fallback_dir)?;
        println!(
            "Moved execution context to {:?} to allow deletion.",
            fallback_dir
        );
    }

    let cache_dir = utils::paths::get_bare_repos_dir()?;
    for (worktree_name, repo_url) in &manifest.repositories {
        let worktree_path = workspace_root.join(worktree_name);
        if worktree_path.is_dir() && worktree_path.join(".git").exists() {
            let bare_repo_path = bare_repo_path(&cache_dir, repo_url);
            if bare_repo_path.is_dir() {
                git::engine::remove_worktree(&bare_repo_path, &worktree_path).await?;
            }
        }
    }

    fs::remove_dir_all(&workspace_root)?;
    println!("Deleted workspace at {:?}", workspace_root);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Basic setup for tracing could go here
    let _guard = if let Ok(state_dir) = utils::paths::get_state_dir() {
        Some(utils::logging::init_logging(&state_dir))
    } else {
        None
    };

    let cli = Cli::parse();

    match cli.command {
        Commands::Init => handle_init().await?,
        Commands::Create { name } => handle_create(&name).await?,
        Commands::Edit { name } => handle_edit(name).await?,
        Commands::Add { target } => handle_add(&target).await?,
        Commands::Delete { name } => handle_delete(name).await?,
    }

    Ok(())
}
