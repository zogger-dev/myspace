use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "myspace")]
#[command(about = "Multi-repo workspace manager using Git worktrees", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Bootstraps the current directory as an empty space
    Init,

    /// Provisions a new workspace directory structure and manifest
    Create { name: String },

    /// Opens the workspace manifest in $EDITOR
    Edit {
        name: Option<String>,
        /// Edit the space .gitignore instead of the manifest
        #[arg(long)]
        ignore: bool,
    },

    /// Resolves the target, clones, and provisions a worktree
    Add { target: String },

    /// Removes a repository worktree from the workspace
    Remove {
        name: String,
        /// Remove even if the worktree has uncommitted changes
        #[arg(long)]
        force: bool,
    },

    /// Safely tears down worktrees and deletes the workspace directory
    Delete { name: Option<String> },

    /// Fetches the space's cached repos and refreshes its views
    Sync {
        /// Sync every registered space instead of the current one
        #[arg(long)]
        all: bool,
    },

    /// Creates or grows a branch set; -d tears one down
    Branch {
        name: String,
        /// Repos joining the set (default: every member of the space)
        repos: Vec<String>,
        /// Stack on another branch set instead of trunk
        #[arg(long)]
        from: Option<String>,
        /// Delete the branch set
        #[arg(short = 'd', long)]
        delete: bool,
        /// With -d: force removal despite dirty worktrees or unpushed commits
        #[arg(long)]
        force: bool,
        /// With -d: keep the git branches, remove only the worktrees
        #[arg(long)]
        keep_branch: bool,
    },

    /// Points the space's canonical views at a branch set, or back at trunk
    View { target: String },

    /// Shows spaces, members, branch sets, views, and their state
    Status {
        /// Report every registered space instead of the current one
        #[arg(long)]
        all: bool,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
}
