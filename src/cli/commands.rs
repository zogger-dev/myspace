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
    /// Bootstraps an empty myspace.toml in the current directory
    Init,

    /// Provisions a new workspace directory structure and manifest
    Create { name: String },

    /// Opens the workspace manifest in $EDITOR
    Edit { name: Option<String> },

    /// Resolves the target, clones, and provisions a worktree
    Add { target: String },

    /// Safely tears down worktrees and deletes the workspace directory
    Delete { name: Option<String> },
}
