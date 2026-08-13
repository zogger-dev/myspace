use anyhow::{Context as _, Result};
use std::fs::{self, File, OpenOptions};
use std::path::Path;

/// An exclusive advisory OS file lock. The OS releases it when the handle is
/// dropped, including on crash, so stale locks cannot wedge anything.
///
/// Lock ordering: always acquire a space lock before any repo lock — every
/// call path follows space → repo, so the two can never deadlock.
pub struct FileLock {
    _file: File,
}

fn lock_file_at(lock_path: &Path) -> Result<FileLock> {
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(lock_path)
        .with_context(|| format!("failed to open lock file {:?}", lock_path))?;
    file.lock()
        .with_context(|| format!("failed to lock {:?}", lock_path))?;
    Ok(FileLock { _file: file })
}

/// Blocks until an exclusive lock on one bare-repo cache entry is held
/// (sibling `<repo>.lock` file). Guards clone/fetch/worktree mutations of
/// that cache entry against concurrent invocations.
pub fn lock_repo(cache_path: &Path) -> Result<FileLock> {
    let parent = cache_path
        .parent()
        .context("cache path has no parent directory")?;
    let file_name = cache_path
        .file_name()
        .context("cache path has no file name")?
        .to_string_lossy();
    lock_file_at(&parent.join(format!("{}.lock", file_name)))
}

/// Blocks until an exclusive lock on a space is held (`.myspace/lock`).
/// Guards the manifest/state load→mutate→save sequences: without it, two
/// concurrent commands editing different repos would each save a stale
/// manifest snapshot, and the last write would silently drop the other's
/// change.
pub fn lock_space(space_root: &Path) -> Result<FileLock> {
    lock_file_at(&space_root.join(".myspace").join("lock"))
}
