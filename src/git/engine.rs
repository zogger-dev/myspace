use anyhow::{Result, anyhow};
use indicatif::ProgressBar;
use std::path::Path;
use git2::{Repository, FetchOptions, build::RepoBuilder, RemoteCallbacks, FetchPrune, Cred};

fn create_fetch_options<'a>(spinner: Option<&'a ProgressBar>) -> FetchOptions<'a> {
    let mut callbacks = RemoteCallbacks::new();
    if let Some(spinner) = spinner {
        let spinner_clone = spinner.clone();
        callbacks.transfer_progress(move |stats| {
            spinner_clone.set_message(format!(
                "Receiving objects: {}/{} ({} bytes)",
                stats.received_objects(),
                stats.total_objects(),
                stats.received_bytes()
            ));
            true
        });
    }

    callbacks.credentials(|_user, username_from_url, _allowed_types| {
        Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
    });

    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);
    fetch_options.prune(FetchPrune::On);
    fetch_options
}

/// Bare clones a git repository into the specified cache directory.
pub async fn clone_bare(repo_url: &str, cache_path: &Path, spinner: Option<&ProgressBar>) -> Result<()> {
    if cache_path.exists() {
        let repo_url = repo_url.to_string();
        let cache_path = cache_path.to_path_buf();
        let spinner_cloned = spinner.cloned();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let repo = Repository::open_bare(&cache_path)?;
            let mut remote = repo.find_remote("origin")
                .or_else(|_| repo.remote_anonymous(&repo_url))?;
            let mut fetch_options = create_fetch_options(spinner_cloned.as_ref());
            remote.fetch(&["+refs/heads/*:refs/heads/*"], Some(&mut fetch_options), None)?;
            Ok(())
        }).await??;

        return Ok(());
    }

    let repo_url = repo_url.to_string();
    let cache_path = cache_path.to_path_buf();
    let spinner_cloned = spinner.cloned();

    tokio::task::spawn_blocking(move || -> Result<()> {
        let fetch_options = create_fetch_options(spinner_cloned.as_ref());
        let mut builder = RepoBuilder::new();
        builder
            .bare(true)
            .fetch_options(fetch_options);
        builder.clone(&repo_url, &cache_path)?;
        Ok(())
    }).await??;

    Ok(())
}

/// Creates a new worktree from a bare repository into a target path.
pub async fn add_worktree(bare_repo_path: &Path, worktree_path: &Path, _spinner: Option<&ProgressBar>) -> Result<()> {
    let status = tokio::process::Command::new("git")
        .arg("worktree")
        .arg("add")
        .arg("--detach")
        .arg(worktree_path)
        .current_dir(bare_repo_path)
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
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
    let status = tokio::process::Command::new("git")
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(worktree_path)
        .current_dir(bare_repo_path)
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status()
        .await?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("Failed to remove worktree at {:?}", worktree_path))
    }
}
