use anyhow::Result;
use clap::Parser;
use myspace::cli::commands::{Cli, Commands};
use myspace::ops::{self, Context};
use myspace::utils;

fn main() -> Result<()> {
    let _guard = if let Ok(logs_dir) = utils::paths::get_logs_dir() {
        Some(utils::logging::init_logging(&logs_dir))
    } else {
        None
    };

    let cli = Cli::parse();
    let ctx = Context::from_env()?;

    match cli.command {
        Commands::Init => ops::init(&ctx)?,
        Commands::Create { name } => ops::create(&ctx, &name)?,
        Commands::Edit { name, ignore } => ops::edit(&ctx, name.as_deref(), ignore)?,
        Commands::Add { target } => ops::add(&ctx, &target)?,
        Commands::Remove { name, force } => ops::remove(&ctx, &name, force)?,
        Commands::Delete { name } => ops::delete(&ctx, name.as_deref())?,
        Commands::Sync { all } => ops::sync(&ctx, all)?,
        Commands::Branch {
            name,
            repos,
            from,
            delete,
            force,
            keep_branch,
        } => {
            if delete {
                if !repos.is_empty() || from.is_some() {
                    anyhow::bail!("-d takes only the branch set name");
                }
                ops::branch_delete(&ctx, &name, force, keep_branch)?;
            } else {
                if force || keep_branch {
                    anyhow::bail!("--force and --keep-branch only apply with -d");
                }
                ops::branch(&ctx, &name, &repos, from.as_deref())?;
            }
        }
        Commands::View { target } => ops::view(&ctx, &target)?,
        Commands::Status { all, json } => ops::status(&ctx, all, json)?,
    }

    Ok(())
}
