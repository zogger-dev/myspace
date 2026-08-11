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
use std::env;
use std::fs;
use tokio::process::Command;
use utils::context::find_workspace_root;

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
        let mut p = env::current_dir()?;
        p.push(n);
        p.push("myspace.toml");
        p
    } else {
        find_workspace_root()
            .ok_or_else(|| anyhow!("Not in a myspace workspace"))?
            .join("myspace.toml")
    };

    if !manifest_path.exists() {
        return Err(anyhow!("Manifest not found at {:?}", manifest_path));
    }

    let editor = env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    Command::new(editor).arg(&manifest_path).status().await?;

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
    // For simplicity, just use the hash of the URL or a sanitised version.
    let repo_name = git_url.replace("/", "_").replace(":", "_");
    let bare_repo_path = cache_dir.join(&repo_name);

    git::engine::clone_bare(&git_url, &bare_repo_path).await?;

    let worktree_name = match &parsed_target {
        models::target::Target::Bare(repo) => repo.clone(),
        models::target::Target::OrgRepo(_, repo) => repo.clone(),
        models::target::Target::Aliased(_, _, repo) => repo.clone(),
    };

    let worktree_path = workspace_root.join(&worktree_name);

    spinner.set_message(format!("Provisioning worktree {}...", worktree_name));
    git::engine::add_worktree(&bare_repo_path, &worktree_path).await?;

    let manifest_path = workspace_root.join("myspace.toml");
    let mut manifest = WorkspaceManifest::load(&manifest_path)?;
    manifest.repositories.insert(worktree_name.clone(), git_url);
    manifest.save(&manifest_path)?;

    spinner.finish_with_message(format!("Successfully added {} to workspace", worktree_name));
    Ok(())
}

async fn handle_delete(name: Option<String>) -> Result<()> {
    let workspace_root = if let Some(n) = name {
        let mut p = env::current_dir()?;
        p.push(n);
        p
    } else {
        find_workspace_root().ok_or_else(|| anyhow!("Not in a myspace workspace"))?
    };

    if !workspace_root.exists() {
        return Err(anyhow!("Workspace not found at {:?}", workspace_root));
    }

    let current_dir = env::current_dir()?;
    if current_dir.starts_with(&workspace_root) {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot find home directory"))?;
        env::set_current_dir(&home)?;
        println!("Moved execution context to home directory to allow deletion.");
    }

    // Read subdirectories to find git worktrees and remove them via git worktree remove
    let cache_dir = utils::paths::get_bare_repos_dir()?;
    let entries = fs::read_dir(&workspace_root)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() && path.join(".git").exists() {
            // Find which bare repo this belongs to. Since we don't store it explicitly,
            // we will search the cache directory for a bare repo that lists this worktree.
            // A more robust solution would store this in `myspace.toml`.
            // For now, we iterate over the bare repos and try to remove the worktree.
            if let Ok(bare_repos) = fs::read_dir(&cache_dir) {
                for bare_repo_entry in bare_repos.flatten() {
                    let bare_repo_path = bare_repo_entry.path();
                    if bare_repo_path.is_dir() {
                        let _ = git::engine::remove_worktree(&bare_repo_path, &path).await;
                    }
                }
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
