use anyhow::{Result, anyhow};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// Bare clones a git repository into the specified cache directory.
pub async fn clone_bare(repo_url: &str, cache_path: &Path) -> Result<()> {
    if cache_path.exists() {
        // If it exists, let's try to update it instead
        let status = Command::new("git")
            .arg("fetch")
            .arg(repo_url)
            .arg("+refs/heads/*:refs/heads/*")
            .arg("--prune")
            .current_dir(cache_path)
            .status()
            .await?;

        if !status.success() {
            return Err(anyhow!(
                "Failed to fetch updates for bare repo at {:?}",
                cache_path
            ));
        }
        return Ok(());
    }

    let status = Command::new("git")
        .arg("clone")
        .arg("--bare")
        .arg(repo_url)
        .arg(cache_path)
        .status()
        .await?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("Failed to clone bare repository {}", repo_url))
    }
}

/// Creates a new worktree from a bare repository into a target path.
pub async fn add_worktree(bare_repo_path: &Path, worktree_path: &Path) -> Result<()> {
    let status = Command::new("git")
        .arg("worktree")
        .arg("add")
        .arg("--detach")
        .arg(worktree_path)
        .current_dir(bare_repo_path)
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .await?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("Failed to add worktree at {:?}", worktree_path))
    }
}

/// Removes a worktree from a bare repository.
pub async fn remove_worktree(bare_repo_path: &Path, worktree_path: &Path) -> Result<()> {
    let status = Command::new("git")
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(worktree_path)
        .current_dir(bare_repo_path)
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .await?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("Failed to remove worktree at {:?}", worktree_path))
    }
}
