use std::path::{Path, PathBuf};

/// Detects whether `start` is inside a space by looking for a
/// `.myspace/config.toml` marker, traversing upwards to the filesystem root.
pub fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut current = start;
    loop {
        if current.join(".myspace").join("config.toml").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}
