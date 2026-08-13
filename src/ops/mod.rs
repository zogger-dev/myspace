use crate::engine::target_parser::{self, ResolvedRepo, is_valid_component};
use crate::git;
use crate::models::config::GlobalConfig;
use crate::models::manifest::{WorkspaceDetails, WorkspaceManifest};
use crate::models::state::SpaceState;
use crate::utils::context::find_workspace_root;
use crate::utils::lock;
use crate::utils::registry;
use anyhow::{Result, anyhow};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Ambient state for command execution, passed explicitly so commands can be
/// exercised against temp directories in tests instead of the real cwd,
/// config, and cache locations.
pub struct Context {
    pub cwd: PathBuf,
    pub config_path: PathBuf,
    pub cache_dir: PathBuf,
    pub registry_path: PathBuf,
}

impl Context {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            cwd: env::current_dir()?,
            config_path: crate::utils::paths::get_config_path()?,
            cache_dir: crate::utils::paths::get_bare_repos_dir()?,
            registry_path: crate::utils::paths::myspace_home()?.join("spaces.toml"),
        })
    }
}

/// All per-space machinery lives under `<space>/.myspace/`: config.toml (the
/// manifest — the shareable definition), state.toml (local view state),
/// trees/ (branch-set worktrees), deps/ (pinned dependency checkouts). The
/// space root itself shows only the trunk repo views.
const SPACE_DIR: &str = ".myspace";

fn space_config_path(space_root: &Path) -> PathBuf {
    space_root.join(SPACE_DIR).join("config.toml")
}

fn space_state_path(space_root: &Path) -> PathBuf {
    space_root.join(SPACE_DIR).join("state.toml")
}

fn space_trees_dir(space_root: &Path) -> PathBuf {
    space_root.join(SPACE_DIR).join("trees")
}

/// Manifest identities are hand-editable; refuse anything that isn't a clean
/// `host/org/repo` before joining it into a filesystem path.
fn validate_identity(identity: &str) -> Result<()> {
    let parts: Vec<&str> = identity.split('/').collect();
    if parts.len() != 3 || parts.iter().any(|p| !is_valid_component(p)) {
        return Err(anyhow!(
            "Manifest contains invalid repo identity '{}' — fix it with `myspace edit`",
            identity
        ));
    }
    Ok(())
}

/// The space's branch namespace prefix, from the manifest's space name.
fn space_branch(manifest: &WorkspaceManifest, name: &str) -> Result<String> {
    let space = &manifest.workspace.name;
    if !is_valid_component(space) {
        return Err(anyhow!(
            "Space name '{}' is not usable as a branch namespace — fix it with `myspace edit`",
            space
        ));
    }
    Ok(format!("{}/{}", space, name))
}

fn workspace_root(ctx: &Context) -> Result<PathBuf> {
    find_workspace_root(&ctx.cwd).ok_or_else(|| anyhow!("Not in a myspace workspace"))
}

const GITIGNORE_BEGIN: &str = "# --- managed by myspace: begin (do not edit inside this block) ---";
const GITIGNORE_END: &str = "# --- managed by myspace: end ---";

/// Regenerates the myspace-managed block of the space's `.gitignore`,
/// preserving any user content outside the markers. Ignoring the member
/// repo dirs (nested git repos) and local-only state makes the space root
/// safely `git init`-able to version the manifest.
fn write_managed_gitignore(space_root: &Path, manifest: &WorkspaceManifest) -> Result<()> {
    let mut block = String::new();
    block.push_str(GITIGNORE_BEGIN);
    block.push('\n');
    for entry in [".myspace/state.toml", ".myspace/trees/", ".myspace/deps/"] {
        block.push_str(entry);
        block.push('\n');
    }
    for name in manifest.repositories.keys() {
        block.push_str(&format!("/{}/\n", name));
    }
    block.push_str(GITIGNORE_END);
    block.push('\n');

    let path = space_root.join(".gitignore");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let content = match (existing.find(GITIGNORE_BEGIN), existing.find(GITIGNORE_END)) {
        (Some(start), Some(end_marker)) if end_marker >= start => {
            let end = existing[end_marker..]
                .find('\n')
                .map(|n| end_marker + n + 1)
                .unwrap_or(existing.len());
            format!("{}{}{}", &existing[..start], block, &existing[end..])
        }
        _ if existing.is_empty() => block,
        _ => {
            let mut content = existing;
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push('\n');
            content.push_str(&block);
            content
        }
    };
    fs::write(&path, content)?;
    Ok(())
}

fn validate_workspace_subpath(name: &str) -> Result<()> {
    let name_path = Path::new(name);
    if name_path.is_absolute()
        || name_path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(anyhow!(
            "Workspace name must be a relative path under the current directory"
        ));
    }
    Ok(())
}

fn resolve_named_workspace(ctx: &Context, name: &str) -> Result<PathBuf> {
    validate_workspace_subpath(name)?;
    let cwd = ctx.cwd.canonicalize()?;
    let workspace_root = cwd.join(name);
    if !workspace_root.exists() {
        return Err(anyhow!("Workspace not found at {:?}", workspace_root));
    }
    let workspace_root = workspace_root.canonicalize()?;
    if !workspace_root.starts_with(&cwd) {
        return Err(anyhow!(
            "Workspace path must stay within the current directory"
        ));
    }

    Ok(workspace_root)
}

fn init_space_at(space_root: &Path, name: &str, description: String) -> Result<()> {
    let manifest_path = space_config_path(space_root);
    fs::create_dir_all(manifest_path.parent().expect("space config has a parent"))?;

    let manifest = WorkspaceManifest {
        workspace: WorkspaceDetails {
            name: name.to_string(),
            description: Some(description),
        },
        repositories: BTreeMap::new(),
    };
    manifest.save(&manifest_path)?;
    write_managed_gitignore(space_root, &manifest)?;
    Ok(())
}

pub fn init(ctx: &Context) -> Result<()> {
    if space_config_path(&ctx.cwd).exists() {
        return Err(anyhow!("The current directory is already a myspace space"));
    }

    let name = ctx
        .cwd
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    init_space_at(&ctx.cwd, &name, "Auto-generated workspace".to_string())?;
    registry::register_space(&ctx.registry_path, &ctx.cwd)?;
    println!("Initialized myspace workspace in {:?}", ctx.cwd);
    Ok(())
}

pub fn create(ctx: &Context, name: &str) -> Result<()> {
    validate_workspace_subpath(name)?;
    let workspace_dir = ctx.cwd.join(name);

    if workspace_dir.exists() {
        return Err(anyhow!("Directory {} already exists", name));
    }

    fs::create_dir_all(&workspace_dir)?;
    init_space_at(&workspace_dir, name, format!("Workspace {}", name))?;
    registry::register_space(&ctx.registry_path, &workspace_dir)?;
    println!("Created workspace {} at {:?}", name, workspace_dir);
    Ok(())
}

pub fn edit(ctx: &Context, name: Option<&str>, ignore: bool) -> Result<()> {
    let space_root = match name {
        Some(n) => resolve_named_workspace(ctx, n)?,
        None => workspace_root(ctx)?,
    };
    let target_path = if ignore {
        space_root.join(".gitignore")
    } else {
        space_config_path(&space_root)
    };

    if !ignore && !target_path.exists() {
        return Err(anyhow!("Manifest not found at {:?}", target_path));
    }

    let editor = env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    let status = std::process::Command::new(&editor)
        .arg(&target_path)
        .status()?;
    if !status.success() {
        return Err(anyhow!("Editor '{}' exited unsuccessfully", editor));
    }

    // Catch manifest syntax errors now rather than on the next command.
    if !ignore {
        WorkspaceManifest::load(&target_path)
            .map_err(|e| anyhow!("Manifest is invalid after editing: {}", e))?;
    }

    Ok(())
}

pub fn add(ctx: &Context, target: &str) -> Result<()> {
    let root = workspace_root(ctx)?;

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );
    spinner.set_message(format!("Resolving target {}...", target));
    spinner.enable_steady_tick(std::time::Duration::from_millis(120));

    let global_config = GlobalConfig::load(&ctx.config_path)?;
    let parsed_target = target_parser::parse_target(target)?;
    let resolved = target_parser::resolve(&parsed_target, &global_config)?;
    let ssh_key = global_config.ssh_key_for(&resolved.host);

    add_resolved(ctx, &root, &resolved, ssh_key.as_deref(), Some(&spinner))?;

    spinner.finish_with_message(format!("Successfully added {} to workspace", resolved.repo));
    Ok(())
}

/// Clones/refreshes the cached bare repo, provisions the worktree, and
/// records it in the manifest. Split from `add` so tests can drive the full
/// flow with a locally constructed [`ResolvedRepo`].
pub fn add_resolved(
    ctx: &Context,
    workspace_root: &Path,
    resolved: &ResolvedRepo,
    ssh_key: Option<&Path>,
    spinner: Option<&ProgressBar>,
) -> Result<()> {
    if resolved.git_ref.is_some() {
        return Err(anyhow!(
            "@ref pins apply to dependency checkouts (--dep), which aren't implemented yet"
        ));
    }

    let worktree_name = resolved.repo.clone();
    let worktree_path = workspace_root.join(&worktree_name);
    let manifest_path = space_config_path(workspace_root);
    let mut manifest = WorkspaceManifest::load(&manifest_path)?;
    let identity = resolved.identity();

    // Fail before any cloning: a second repo with the same basename (e.g.
    // `add other-org/impala` after `add impala`) would otherwise collide.
    if let Some(existing) = manifest.repositories.get(&worktree_name)
        && existing != &identity
    {
        return Err(anyhow!(
            "'{}' already tracks {} — remove it first or pick a different repo",
            worktree_name,
            existing
        ));
    }
    if worktree_path.exists() {
        return Err(anyhow!(
            "{:?} already exists in the workspace",
            worktree_path
        ));
    }

    let cache_path = resolved.cache_path(&ctx.cache_dir);
    let _lock = lock::lock_repo(&cache_path)?;

    if let Some(spinner) = spinner {
        spinner.set_message(format!("Cloning bare repository from {}...", resolved.url));
    }
    git::engine::clone_bare(&resolved.url, &cache_path, ssh_key, spinner)?;

    if let Some(spinner) = spinner {
        spinner.set_message(format!("Provisioning worktree {}...", worktree_name));
    }
    git::engine::add_worktree(&cache_path, &worktree_path, None)?;

    manifest.repositories.insert(worktree_name, identity);
    if let Err(e) = manifest.save(&manifest_path) {
        // Roll back the worktree so disk state doesn't diverge from the manifest.
        let _ = git::engine::remove_worktree(&cache_path, &worktree_path, true);
        return Err(e);
    }
    write_managed_gitignore(workspace_root, &manifest)?;
    Ok(())
}

pub fn remove(ctx: &Context, name: &str, force: bool) -> Result<()> {
    if !is_valid_component(name) {
        return Err(anyhow!("Invalid repository name '{}'", name));
    }
    let root = workspace_root(ctx)?;
    let manifest_path = space_config_path(&root);
    let mut manifest = WorkspaceManifest::load(&manifest_path)?;

    if !manifest.repositories.contains_key(name) {
        return Err(anyhow!("'{}' is not tracked in this workspace", name));
    }

    let worktree_path = root.join(name);
    if worktree_path.exists() {
        remove_worktree_dir(&worktree_path, force)?;
    }

    // Remove the manifest entry even if the directory was already gone, so
    // `remove` doubles as reconciliation for hand-deleted worktrees.
    manifest.repositories.remove(name);
    manifest.save(&manifest_path)?;
    write_managed_gitignore(&root, &manifest)?;
    println!("Removed {} from workspace", name);
    Ok(())
}

/// Removes a worktree directory, deregistering it from its owning bare repo
/// when it is a linked worktree, or falling back to a plain delete when the
/// bare repo is gone.
fn remove_worktree_dir(worktree_path: &Path, force: bool) -> Result<()> {
    match git::engine::owning_bare_repo(worktree_path).filter(|p| p.is_dir()) {
        Some(bare_repo_path) => {
            let _lock = lock::lock_repo(&bare_repo_path)?;
            git::engine::remove_worktree(&bare_repo_path, worktree_path, force)
        }
        None => Ok(fs::remove_dir_all(worktree_path)?),
    }
}

pub fn delete(ctx: &Context, name: Option<&str>) -> Result<()> {
    let workspace_root = match name {
        Some(n) => resolve_named_workspace(ctx, n)?,
        None => workspace_root(ctx)?.canonicalize()?,
    };

    let manifest_path = space_config_path(&workspace_root);
    if !manifest_path.exists() {
        return Err(anyhow!("Manifest not found at {:?}", manifest_path));
    }
    let manifest = WorkspaceManifest::load(&manifest_path)?;

    // On Windows a process cannot delete its own cwd, and on any platform the
    // user's shell would be left in a dead directory — move out first.
    let current_dir = env::current_dir()?.canonicalize()?;
    if current_dir.starts_with(&workspace_root) {
        let mut fallback_dirs = Vec::new();
        if let Some(home) = dirs::home_dir().and_then(|path| path.canonicalize().ok()) {
            fallback_dirs.push(home);
        }
        if let Some(parent) = workspace_root
            .parent()
            .and_then(|path| path.canonicalize().ok())
        {
            fallback_dirs.push(parent);
        }
        fallback_dirs.push(env::temp_dir());

        let fallback_dir = fallback_dirs
            .into_iter()
            .find(|path| !path.starts_with(&workspace_root))
            .ok_or_else(|| anyhow!("Cannot determine a safe directory outside the workspace"))?;
        env::set_current_dir(&fallback_dir)?;
        println!(
            "Moved execution context to {:?} to allow deletion.",
            fallback_dir
        );
    }

    for worktree_name in manifest.repositories.keys() {
        // Names come from the (hand-editable) manifest; refuse anything that
        // could escape the workspace directory.
        if !is_valid_component(worktree_name) {
            return Err(anyhow!(
                "Manifest contains invalid repository name '{}' — fix it with `myspace edit`",
                worktree_name
            ));
        }
        let worktree_path = workspace_root.join(worktree_name);
        if worktree_path.exists() {
            remove_worktree_dir(&worktree_path, true)?;
        }
    }

    fs::remove_dir_all(&workspace_root)?;
    registry::unregister_space(&ctx.registry_path, &workspace_root)?;
    println!("Deleted workspace at {:?}", workspace_root);

    Ok(())
}

/// Creates or grows a branch set: for each repo, a git branch namespaced as
/// `<space>/<name>` plus a worktree holding it under `.myspace/trees/<name>/`.
/// With no repos given, the whole space joins — worktrees are cheap, and an
/// agent discovering a missing repo mid-task is not. Idempotent: re-running
/// grows the set.
pub fn branch(ctx: &Context, name: &str, repos: &[String], from: Option<&str>) -> Result<()> {
    if !is_valid_component(name) {
        return Err(anyhow!("Invalid branch set name '{}'", name));
    }
    if let Some(from) = from
        && !is_valid_component(from)
    {
        return Err(anyhow!("Invalid --from name '{}'", from));
    }

    let root = workspace_root(ctx)?;
    let manifest = WorkspaceManifest::load(&space_config_path(&root))?;
    let full_branch = space_branch(&manifest, name)?;
    let set_dir = space_trees_dir(&root).join(name);

    if manifest.repositories.is_empty() {
        return Err(anyhow!(
            "This space has no repos — `myspace add` some first"
        ));
    }
    let repos: Vec<String> = if repos.is_empty() {
        manifest.repositories.keys().cloned().collect()
    } else {
        repos.to_vec()
    };

    for repo_name in &repos {
        let identity = manifest.repositories.get(repo_name).ok_or_else(|| {
            anyhow!(
                "'{}' is not a member of this space — `myspace add` it first",
                repo_name
            )
        })?;
        validate_identity(identity)?;

        let worktree_path = set_dir.join(repo_name);
        if worktree_path.exists() {
            continue; // already in the set
        }

        let cache_path = ctx.cache_dir.join(identity);
        if !cache_path.is_dir() {
            return Err(anyhow!(
                "No cached repo for '{}' ({}) — run `myspace sync` first",
                repo_name,
                identity
            ));
        }
        let _lock = lock::lock_repo(&cache_path)?;

        let base = match from {
            Some(from) => {
                let from_branch = space_branch(&manifest, from)?;
                git::engine::branch_tip(&cache_path, &from_branch).map_err(|e| {
                    anyhow!(
                        "cannot stack '{}' on '{}' for repo '{}': {}",
                        name,
                        from,
                        repo_name,
                        e
                    )
                })?
            }
            None => git::engine::trunk_tip(&cache_path)?,
        };

        git::engine::add_worktree_on_branch(&cache_path, &worktree_path, &full_branch, base)?;
        println!("  {} -> {}", repo_name, worktree_path.display());
    }

    println!(
        "Branch set '{}' ({}) at {}",
        name,
        full_branch,
        set_dir.display()
    );
    Ok(())
}

/// Tears down a branch set: removes its worktrees and (unless `keep_branch`)
/// deletes the namespaced branches. Without `force`, dirty worktrees and
/// unmerged/unpushed branches are refused.
pub fn branch_delete(ctx: &Context, name: &str, force: bool, keep_branch: bool) -> Result<()> {
    if !is_valid_component(name) {
        return Err(anyhow!("Invalid branch set name '{}'", name));
    }
    let root = workspace_root(ctx)?;
    let manifest = WorkspaceManifest::load(&space_config_path(&root))?;
    let full_branch = space_branch(&manifest, name)?;
    let set_dir = space_trees_dir(&root).join(name);

    if !set_dir.is_dir() {
        return Err(anyhow!("No branch set named '{}' in this space", name));
    }

    let mut entries: Vec<PathBuf> = fs::read_dir(&set_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    entries.sort();

    for worktree_path in entries {
        match git::engine::owning_bare_repo(&worktree_path).filter(|p| p.is_dir()) {
            Some(bare_repo_path) => {
                if !force {
                    git::engine::ensure_branch_disposable(&bare_repo_path, &full_branch)?;
                }
                let _lock = lock::lock_repo(&bare_repo_path)?;
                git::engine::remove_worktree(&bare_repo_path, &worktree_path, force)?;
                if !keep_branch {
                    git::engine::delete_branch(&bare_repo_path, &full_branch)?;
                }
            }
            None if force => fs::remove_dir_all(&worktree_path)?,
            None => {
                return Err(anyhow!(
                    "{:?} is not a linked worktree — use --force to delete anyway",
                    worktree_path
                ));
            }
        }
    }
    fs::remove_dir_all(&set_dir)?;

    // If the space was viewing this set, fall back to trunk.
    let state_path = space_state_path(&root);
    let mut state = SpaceState::load(&state_path)?;
    if state.view.as_deref() == Some(name) {
        state.view = None;
        state.save(&state_path)?;
        let warnings = refresh_space_views(&root, &manifest, &None)?;
        report_view_warnings(&warnings);
        println!("View reset to trunk");
    }

    println!("Deleted branch set '{}'", name);
    Ok(())
}

/// Points the space's canonical (root) views at a branch set's tips, or back
/// at trunk. Repos not in the set stay at trunk. The selection persists in
/// state.toml so `sync` keeps refreshing the active view.
pub fn view(ctx: &Context, target: &str) -> Result<()> {
    let root = workspace_root(ctx)?;
    let manifest = WorkspaceManifest::load(&space_config_path(&root))?;

    let view = if target == "trunk" {
        None
    } else {
        if !is_valid_component(target) {
            return Err(anyhow!("Invalid view target '{}'", target));
        }
        if !space_trees_dir(&root).join(target).is_dir() {
            return Err(anyhow!(
                "No branch set named '{}' in this space (use `myspace branch {} <repos>` to create it, or `myspace view trunk`)",
                target,
                target
            ));
        }
        Some(target.to_string())
    };

    let state_path = space_state_path(&root);
    let mut state = SpaceState::load(&state_path)?;
    state.view = view.clone();
    state.save(&state_path)?;

    let warnings = refresh_space_views(&root, &manifest, &view)?;
    report_view_warnings(&warnings);
    println!("Space now viewing {}", view.as_deref().unwrap_or("trunk"));
    Ok(())
}

/// Re-detaches every clean member view at its current target (trunk, or the
/// active branch set's tip for repos in the set). Dirty views are reported,
/// never touched. Returns the warnings.
fn refresh_space_views(
    space_root: &Path,
    manifest: &WorkspaceManifest,
    view: &Option<String>,
) -> Result<Vec<String>> {
    let space = &manifest.workspace.name;
    let mut warnings = Vec::new();
    for repo_name in manifest.repositories.keys() {
        if !is_valid_component(repo_name) {
            warnings.push(format!("{}: invalid name in manifest, skipped", repo_name));
            continue;
        }
        let worktree_path = space_root.join(repo_name);
        if !worktree_path.is_dir() {
            continue;
        }
        let Some(bare_repo_path) =
            git::engine::owning_bare_repo(&worktree_path).filter(|p| p.is_dir())
        else {
            warnings.push(format!("{}: not a linked worktree, skipped", repo_name));
            continue;
        };

        let target = match view {
            Some(set)
                if space_trees_dir(space_root)
                    .join(set)
                    .join(repo_name)
                    .is_dir() =>
            {
                git::engine::branch_tip(&bare_repo_path, &format!("{}/{}", space, set))?
            }
            _ => git::engine::trunk_tip(&bare_repo_path)?,
        };

        if let Err(e) = git::engine::retarget_worktree(&worktree_path, target) {
            warnings.push(format!("{}: {}", repo_name, e));
        }
    }
    Ok(warnings)
}

fn report_view_warnings(warnings: &[String]) {
    for warning in warnings {
        println!("  skipped {}", warning);
    }
}

/// Fetches every cached repo of a space and refreshes its views at the
/// current view target. With `all`, does so for every registered space.
pub fn sync(ctx: &Context, all: bool) -> Result<()> {
    let spaces = if all {
        registry::registered_spaces(&ctx.registry_path)?
    } else {
        vec![workspace_root(ctx)?]
    };
    if spaces.is_empty() {
        println!("No spaces registered");
        return Ok(());
    }

    // Config is optional here: fetching uses each cache's stored origin URL,
    // and without a config we simply fall back to SSH-agent auth.
    let config = GlobalConfig::load(&ctx.config_path).ok();

    let mut failures = Vec::new();
    for space_root in &spaces {
        println!("Syncing {}", space_root.display());
        if let Err(e) = sync_space(ctx, space_root, config.as_ref()) {
            failures.push(format!("{}: {}", space_root.display(), e));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow!("sync failed for:\n  {}", failures.join("\n  ")))
    }
}

/// Snapshot of one space, derived entirely from git metadata + the manifest
/// at call time — nothing here is stored state that could drift.
#[derive(Debug, serde::Serialize)]
pub struct SpaceStatus {
    pub name: String,
    pub root: PathBuf,
    /// "trunk" or the active branch set name.
    pub view: String,
    pub repos: Vec<RepoStatus>,
    pub branch_sets: Vec<BranchSetStatus>,
}

#[derive(Debug, serde::Serialize)]
pub struct RepoStatus {
    pub name: String,
    pub identity: String,
    pub path: PathBuf,
    pub present: bool,
    pub dirty: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct BranchSetStatus {
    pub name: String,
    /// The namespaced git branch, `<space>/<name>`.
    pub branch: String,
    pub repos: Vec<BranchRepoStatus>,
}

#[derive(Debug, serde::Serialize)]
pub struct BranchRepoStatus {
    pub name: String,
    pub path: PathBuf,
    pub dirty: bool,
    /// Commits neither merged into trunk nor pushed to origin.
    pub needs_push: bool,
}

/// Collects the status snapshot for one space.
pub fn space_status(space_root: &Path) -> Result<SpaceStatus> {
    let manifest = WorkspaceManifest::load(&space_config_path(space_root))?;
    let state = SpaceState::load(&space_state_path(space_root))?;
    let space = manifest.workspace.name.clone();

    let mut repos = Vec::new();
    for (name, identity) in &manifest.repositories {
        let path = space_root.join(name);
        let present = path.is_dir();
        let dirty = present && git::engine::worktree_dirty(&path).unwrap_or(false);
        repos.push(RepoStatus {
            name: name.clone(),
            identity: identity.clone(),
            path,
            present,
            dirty,
        });
    }

    let mut branch_sets = Vec::new();
    let trees = space_trees_dir(space_root);
    if trees.is_dir() {
        let mut sets: Vec<PathBuf> = fs::read_dir(&trees)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        sets.sort();
        for set_dir in sets {
            let set_name = set_dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let branch = format!("{}/{}", space, set_name);
            let mut set_repos = Vec::new();
            let mut members: Vec<PathBuf> = fs::read_dir(&set_dir)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            members.sort();
            for worktree_path in members {
                let repo_name = worktree_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let dirty = git::engine::worktree_dirty(&worktree_path).unwrap_or(false);
                let needs_push = git::engine::owning_bare_repo(&worktree_path)
                    .filter(|p| p.is_dir())
                    .map(|bare| git::engine::branch_needs_push(&bare, &branch).unwrap_or(false))
                    .unwrap_or(false);
                set_repos.push(BranchRepoStatus {
                    name: repo_name,
                    path: worktree_path,
                    dirty,
                    needs_push,
                });
            }
            branch_sets.push(BranchSetStatus {
                name: set_name,
                branch,
                repos: set_repos,
            });
        }
    }

    Ok(SpaceStatus {
        name: space,
        root: space_root.to_path_buf(),
        view: state.view.unwrap_or_else(|| "trunk".to_string()),
        repos,
        branch_sets,
    })
}

/// Read-only orientation: spaces, members, branch sets, views, dirt.
/// `--json` emits the same data machine-readably (the agent hook).
pub fn status(ctx: &Context, all: bool, json: bool) -> Result<()> {
    let spaces = if all {
        registry::registered_spaces(&ctx.registry_path)?
    } else {
        vec![workspace_root(ctx)?]
    };

    let statuses: Vec<SpaceStatus> = spaces
        .iter()
        .map(|root| space_status(root))
        .collect::<Result<_>>()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&statuses)?);
        return Ok(());
    }

    for s in &statuses {
        println!(
            "space {} ({}) — viewing {}",
            s.name,
            s.root.display(),
            s.view
        );
        for r in &s.repos {
            let flags = match (r.present, r.dirty) {
                (false, _) => "missing",
                (true, true) => "dirty",
                (true, false) => "clean",
            };
            println!("  {}  {}  {}", r.name, r.identity, flags);
        }
        for set in &s.branch_sets {
            println!("  branch {} ({})", set.name, set.branch);
            for r in &set.repos {
                let mut flags = Vec::new();
                if r.dirty {
                    flags.push("dirty");
                }
                if r.needs_push {
                    flags.push("needs push");
                }
                if flags.is_empty() {
                    flags.push("clean");
                }
                println!(
                    "    {}  {}  [{}]",
                    r.name,
                    r.path.display(),
                    flags.join(", ")
                );
            }
        }
    }
    Ok(())
}

fn sync_space(ctx: &Context, space_root: &Path, config: Option<&GlobalConfig>) -> Result<()> {
    let manifest = WorkspaceManifest::load(&space_config_path(space_root))?;
    let state = SpaceState::load(&space_state_path(space_root))?;

    for (repo_name, identity) in &manifest.repositories {
        validate_identity(identity)?;
        let cache_path = ctx.cache_dir.join(identity);
        if !cache_path.is_dir() {
            println!("  {}: no cache yet, skipped (re-add to clone)", repo_name);
            continue;
        }
        let host = identity.split('/').next().unwrap_or_default();
        let ssh_key = config.and_then(|c| c.ssh_key_for(host));
        let _lock = lock::lock_repo(&cache_path)?;
        git::engine::fetch_bare(&cache_path, ssh_key.as_deref(), None)?;
        println!("  {}: fetched", repo_name);
    }

    let warnings = refresh_space_views(space_root, &manifest, &state.view)?;
    report_view_warnings(&warnings);
    Ok(())
}
