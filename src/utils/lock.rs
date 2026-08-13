use anyhow::{Context as _, Result};
use std::fs::{self, File, OpenOptions};
use std::path::Path;

/// Advisory lock guarding one bare-repo cache entry. Concurrent `myspace`
/// invocations touching the same cached repo (clone/fetch/worktree ops)
/// would otherwise race. The OS releases the lock when the file handle is
/// dropped, including on crash, so stale locks cannot wedge the cache.
pub struct RepoLock {
    _file: File,
}

/// Blocks until an exclusive lock on `<cache_path sibling>.lock` is held.
pub fn lock_repo(cache_path: &Path) -> Result<RepoLock> {
    let parent = cache_path
        .parent()
        .context("cache path has no parent directory")?;
    fs::create_dir_all(parent)?;
    let file_name = cache_path
        .file_name()
        .context("cache path has no file name")?
        .to_string_lossy();
    let lock_path = parent.join(format!("{}.lock", file_name));

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open lock file {:?}", lock_path))?;
    file.lock()
        .with_context(|| format!("failed to lock {:?}", lock_path))?;
    Ok(RepoLock { _file: file })
}
