use std::env;
use std::path::PathBuf;

/// Detects if the current working directory is inside a workspace by looking for a `myspace.toml` file.
/// Traverses upwards to the root.
pub fn find_workspace_root() -> Option<PathBuf> {
    let current_dir = env::current_dir().ok()?;
    let mut current_path = current_dir.as_path();

    loop {
        let manifest_path = current_path.join("myspace.toml");
        if manifest_path.exists() {
            return Some(current_path.to_path_buf());
        }

        if let Some(parent) = current_path.parent() {
            current_path = parent;
        } else {
            break;
        }
    }

    None
}
