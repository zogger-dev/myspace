use anyhow::{Result, anyhow};
use git2::{
    AutotagOption, BranchType, Cred, FetchOptions, FetchPrune, Oid, RemoteCallbacks, Repository,
    StatusOptions, Worktree, WorktreeAddOptions, WorktreePruneOptions, build::RepoBuilder,
};
use indicatif::ProgressBar;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

// This module does all git work through libgit2 (the git2 crate) — no `git`
// binary is required at runtime. It also means git never writes to the
// terminal, so nothing can interleave with the indicatif spinner.

/// The cache fetches into remote-tracking refs, never local heads: local
/// branch refs are reserved for myspace-created branch sets, and a checkout
/// holding a real branch (e.g. a src/ trunk checkout on `main`) must never
/// have that branch moved underneath it by a fetch.
const FETCH_REFSPEC: &str = "+refs/heads/*:refs/remotes/origin/*";

/// One way to authenticate over SSH, tried in order until one succeeds.
enum SshCandidate {
    Agent,
    Key(PathBuf),
}

/// libgit2 knows nothing of `~/.ssh/config`: it can consult the agent at
/// `$SSH_AUTH_SOCK` (see `GlobalConfig::ssh_agent`) or read a key file
/// directly. An explicitly configured key wins outright (IdentitiesOnly
/// semantics); otherwise try the agent, then conventional key paths.
fn ssh_candidates(explicit_key: Option<PathBuf>) -> Vec<SshCandidate> {
    if let Some(key) = explicit_key {
        return vec![SshCandidate::Key(key)];
    }
    let mut candidates = vec![SshCandidate::Agent];
    if let Some(ssh_dir) = dirs::home_dir().map(|home| home.join(".ssh")) {
        for name in [
            "id_ed25519",
            "id_ed25519_sk",
            "id_ecdsa",
            "id_ecdsa_sk",
            "id_rsa",
        ] {
            let path = ssh_dir.join(name);
            if path.is_file() {
                candidates.push(SshCandidate::Key(path));
            }
        }
    }
    candidates
}

/// The `IdentityAgent` path from `~/.ssh/config`, if one is set. Used only
/// to make the auth-failure message actionable — never to drive behavior,
/// so a naive parse is fine here.
fn identity_agent_hint() -> Option<String> {
    let config = fs::read_to_string(dirs::home_dir()?.join(".ssh").join("config")).ok()?;
    for line in config.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Keep scanning past lines without a value — `?` here would abandon
        // the whole search on the first blank or malformed line.
        let Some((key, value)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if key.eq_ignore_ascii_case("identityagent") {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn ssh_auth_help() -> String {
    let mut msg = String::from(
        "SSH authentication failed: no usable identity. libgit2 does not read \
         ~/.ssh/config — it uses the agent at $SSH_AUTH_SOCK or an explicit key file",
    );
    match identity_agent_hint() {
        Some(agent) => msg.push_str(&format!(
            ".\n~/.ssh/config sets IdentityAgent {agent} — add this to ~/.myspace/config.toml:\n\
             \n    ssh_agent = \"{agent}\"\n"
        )),
        None => msg.push_str(
            ".\nLoad a key with `ssh-add <key>`, or set an explicit identity in \
             ~/.myspace/config.toml:\n\n    [hosts.\"github.com\"]\n    \
             ssh_key = \"~/.ssh/id_ed25519\"\n",
        ),
    }
    msg
}

fn create_fetch_options(
    spinner: Option<&ProgressBar>,
    ssh_key: Option<PathBuf>,
) -> FetchOptions<'static> {
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

    // libgit2 re-invokes this callback after each rejected credential, so
    // walk the candidate list one entry per call and fail with guidance once
    // every option is spent.
    let candidates = ssh_candidates(ssh_key);
    let next = Cell::new(0usize);
    callbacks.credentials(move |_url, username_from_url, allowed_types| {
        if allowed_types.contains(git2::CredentialType::SSH_KEY) {
            let user = username_from_url.unwrap_or("git");
            let index = next.get();
            next.set(index + 1);
            return match candidates.get(index) {
                Some(SshCandidate::Agent) => Cred::ssh_key_from_agent(user),
                Some(SshCandidate::Key(path)) => Cred::ssh_key(user, None, path, None),
                None => Err(git2::Error::from_str(&ssh_auth_help())),
            };
        }
        if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT)
            || allowed_types.contains(git2::CredentialType::DEFAULT)
        {
            Cred::default()
        } else {
            Err(git2::Error::from_str("no credentials available"))
        }
    });

    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);
    fetch_options.prune(FetchPrune::On);
    fetch_options.download_tags(AutotagOption::All);
    fetch_options
}

/// Refreshes an existing cached bare repo with a pruning fetch from origin.
pub fn fetch_bare(
    cache_path: &Path,
    ssh_key: Option<&Path>,
    spinner: Option<&ProgressBar>,
) -> Result<()> {
    let repo = Repository::open_bare(cache_path)?;
    let mut remote = repo.find_remote("origin")?;
    let mut fetch_options = create_fetch_options(spinner, ssh_key.map(Path::to_path_buf));
    remote.fetch(&[FETCH_REFSPEC], Some(&mut fetch_options), None)?;
    Ok(())
}

/// Clones a bare repository into the shared cache, or refreshes it with a
/// pruning fetch if it is already cached.
pub fn clone_bare(
    repo_url: &str,
    cache_path: &Path,
    ssh_key: Option<&Path>,
    spinner: Option<&ProgressBar>,
) -> Result<()> {
    if cache_path.exists() {
        fetch_bare(cache_path, ssh_key, spinner)?;
    } else {
        let ssh_key = ssh_key.map(Path::to_path_buf);
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let fetch_options = create_fetch_options(spinner, ssh_key);
        let mut builder = RepoBuilder::new();
        builder
            .bare(true)
            .fetch_options(fetch_options)
            // Override libgit2's bare-clone default (which fetches into
            // local heads) so the cache is remote-tracking from birth.
            .remote_create(|repo, name, url| repo.remote_with_fetch(name, url, FETCH_REFSPEC));
        let repo = builder.clone(repo_url, cache_path)?;
        // Clone finalization creates a local default branch regardless of
        // the refspec; drop the raw ref so local heads stay reserved for
        // branch sets. HEAD remains a symbolic ref to it (unborn, like a
        // fresh init) — exactly what trunk_commit reads to learn the
        // default branch name. Branch::delete would refuse (HEAD guard),
        // so this goes through Reference::delete.
        let head_refs: Vec<String> = repo
            .references_glob("refs/heads/*")?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.name().ok().map(str::to_string))
            .collect();
        for name in head_refs {
            repo.find_reference(&name)?.delete()?;
        }
    }
    Ok(())
}

/// The commit the cache considers trunk: `refs/remotes/origin/<default>`,
/// where the default branch name comes from the bare repo's symbolic HEAD
/// (set from the remote's HEAD at clone time).
pub fn trunk_commit(repo: &Repository) -> Result<Oid> {
    let head = repo.find_reference("HEAD")?;
    let target = head
        .symbolic_target()?
        .ok_or_else(|| anyhow!("Bare repo HEAD is not a symbolic ref"))?;
    let branch = target
        .strip_prefix("refs/heads/")
        .ok_or_else(|| anyhow!("Bare repo HEAD points outside refs/heads: {}", target))?;
    let tracking = repo.find_reference(&format!("refs/remotes/origin/{}", branch))?;
    Ok(tracking.peel_to_commit()?.id())
}

/// Names of all registered worktrees (skipping non-UTF-8 entries).
fn worktree_names(repo: &Repository) -> Result<Vec<String>> {
    Ok(repo
        .worktrees()?
        .iter()
        .flatten()
        .flatten()
        .map(str::to_string)
        .collect())
}

/// Prunes registrations whose working directory no longer exists (e.g. a
/// worktree deleted by hand), so re-adding at the same path works.
fn prune_stale_worktrees(repo: &Repository) -> Result<()> {
    for name in worktree_names(repo)? {
        let worktree = repo.find_worktree(&name)?;
        if !worktree.path().exists() {
            let mut opts = WorktreePruneOptions::new();
            opts.valid(true);
            let _ = worktree.prune(Some(&mut opts));
        }
    }
    Ok(())
}

/// Picks a worktree/branch name that is free in the bare repo. Registrations
/// are keyed by name, so two workspaces holding the same repo need distinct
/// names even though both worktrees are called e.g. `myrepo` on disk.
fn free_worktree_name(repo: &Repository, base: &str) -> Result<String> {
    let existing = worktree_names(repo)?;
    let taken = |name: &str| {
        existing.iter().any(|n| n == name) || repo.find_branch(name, BranchType::Local).is_ok()
    };
    if !taken(base) {
        return Ok(base.to_string());
    }
    for i in 2.. {
        let candidate = format!("{}-{}", base, i);
        if !taken(&candidate) {
            return Ok(candidate);
        }
    }
    unreachable!()
}

/// Creates a new worktree from a bare repository, detached at `at` (or at
/// the trunk commit when `at` is None).
///
/// Detached is deliberate: workspaces share one bare repo per remote, and
/// git refuses to check out the same branch in two worktrees — detached
/// checkouts are the only mode that lets N workspaces hold the same repo.
/// libgit2 has no detach flag on worktree creation, so this creates a
/// scratch branch at the target commit, detaches HEAD there, and deletes
/// the branch again.
pub fn add_worktree(bare_repo_path: &Path, worktree_path: &Path, at: Option<Oid>) -> Result<()> {
    let repo = Repository::open(bare_repo_path)?;
    prune_stale_worktrees(&repo)?;

    let oid = match at {
        Some(oid) => oid,
        None => trunk_commit(&repo)?,
    };

    let base = worktree_path
        .file_name()
        .ok_or_else(|| anyhow!("Worktree path {:?} has no file name", worktree_path))?
        .to_string_lossy();
    let name = free_worktree_name(&repo, &base)?;

    let commit = repo.find_commit(oid)?;
    let scratch = repo.branch(&name, &commit, false)?;
    let created = (|| -> Result<()> {
        let mut opts = WorktreeAddOptions::new();
        opts.reference(Some(scratch.get()));
        let worktree = repo.worktree(&name, worktree_path, Some(&opts))?;

        let worktree_repo = Repository::open_from_worktree(&worktree)?;
        worktree_repo.set_head_detached(oid)?;
        Ok(())
    })();

    if created.is_err() {
        // Roll back so a failed creation cannot leave a scratch branch (the
        // cache invariant reserves local heads for branch sets) or a partial
        // registration behind. Worktree first — a checked-out branch is
        // undeletable.
        if let Ok(partial) = repo.find_worktree(&name) {
            let mut opts = WorktreePruneOptions::new();
            opts.valid(true).working_tree(true).locked(true);
            let _ = partial.prune(Some(&mut opts));
        }
        if let Ok(mut branch) = repo.find_branch(&name, BranchType::Local) {
            let _ = branch.delete();
        }
        return created;
    }

    repo.find_branch(&name, BranchType::Local)?.delete()?;
    Ok(())
}

fn find_worktree_by_path(repo: &Repository, target: &Path) -> Result<Option<Worktree>> {
    let target = target
        .canonicalize()
        .unwrap_or_else(|_| target.to_path_buf());
    for name in worktree_names(repo)? {
        let worktree = repo.find_worktree(&name)?;
        let path = worktree.path();
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if path == target {
            return Ok(Some(worktree));
        }
    }
    Ok(None)
}

/// Removes a worktree (working directory and registration) from a bare
/// repository. Without `force`, a worktree with uncommitted or untracked
/// changes is refused.
pub fn remove_worktree(bare_repo_path: &Path, worktree_path: &Path, force: bool) -> Result<()> {
    let repo = Repository::open(bare_repo_path)?;
    let worktree = find_worktree_by_path(&repo, worktree_path)?.ok_or_else(|| {
        anyhow!(
            "{:?} is not a worktree of {:?}",
            worktree_path,
            bare_repo_path
        )
    })?;

    if !force {
        let worktree_repo = Repository::open(worktree_path)?;
        let mut status_options = StatusOptions::new();
        status_options.include_untracked(true);
        let statuses = worktree_repo.statuses(Some(&mut status_options))?;
        if !statuses.is_empty() {
            return Err(anyhow!(
                "Worktree at {:?} has uncommitted changes; use --force to discard them",
                worktree_path
            ));
        }
    }

    let mut opts = WorktreePruneOptions::new();
    opts.valid(true).working_tree(true).locked(force);
    worktree.prune(Some(&mut opts))?;
    Ok(())
}

/// Tip commit of a local branch in a cached bare repo.
pub fn branch_tip(bare_repo_path: &Path, branch: &str) -> Result<Oid> {
    let repo = Repository::open(bare_repo_path)?;
    let branch = repo
        .find_branch(branch, BranchType::Local)
        .map_err(|e| anyhow!("branch not found: {}", e))?;
    Ok(branch.get().peel_to_commit()?.id())
}

/// Trunk tip of a cached bare repo (path-taking wrapper around
/// [`trunk_commit`]).
pub fn trunk_tip(bare_repo_path: &Path) -> Result<Oid> {
    let repo = Repository::open(bare_repo_path)?;
    trunk_commit(&repo)
}

/// Creates a worktree checked out to a local branch (creating the branch at
/// `at` when it doesn't exist yet). This is the branch-set primitive: unlike
/// the detached views, these worktrees own their branch — git's exclusivity
/// rule then guarantees no other worktree anywhere can hold it.
pub fn add_worktree_on_branch(
    bare_repo_path: &Path,
    worktree_path: &Path,
    branch: &str,
    at: Oid,
) -> Result<()> {
    let repo = Repository::open(bare_repo_path)?;
    prune_stale_worktrees(&repo)?;
    if let Some(parent) = worktree_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let branch_ref = match repo.find_branch(branch, BranchType::Local) {
        Ok(existing) => existing,
        Err(_) => repo.branch(branch, &repo.find_commit(at)?, false)?,
    };

    let base = worktree_path
        .file_name()
        .ok_or_else(|| anyhow!("Worktree path {:?} has no file name", worktree_path))?
        .to_string_lossy();
    let registration = free_worktree_name(&repo, &base)?;

    let mut opts = WorktreeAddOptions::new();
    opts.reference(Some(branch_ref.get()));
    repo.worktree(&registration, worktree_path, Some(&opts))
        .map_err(|e| {
            // Attribute the conflict: git's exclusivity rule means someone
            // already holds this branch — say who, from git's own records.
            match branch_holder(&repo, branch) {
                Some(holder) => anyhow!(
                    "branch '{}' is already checked out at {} — a branch can live in \
                     only one worktree; pick a different name or work there",
                    branch,
                    holder.display()
                ),
                None => e.into(),
            }
        })?;
    Ok(())
}

/// The worktree (if any) that currently has `branch` checked out, resolved
/// from the bare repo's own worktree registrations — git is the database.
fn branch_holder(repo: &Repository, branch: &str) -> Option<PathBuf> {
    let want = format!("refs/heads/{}", branch);
    for name in worktree_names(repo).ok()? {
        let Ok(worktree) = repo.find_worktree(&name) else {
            continue;
        };
        let Ok(wt_repo) = Repository::open(worktree.path()) else {
            continue;
        };
        if let Ok(head) = wt_repo.find_reference("HEAD")
            && head.symbolic_target().ok().flatten() == Some(want.as_str())
        {
            return Some(worktree.path().to_path_buf());
        }
    }
    None
}

/// Whether a worktree has uncommitted or untracked changes.
pub fn worktree_dirty(worktree_path: &Path) -> Result<bool> {
    let repo = Repository::open(worktree_path)?;
    let mut status_options = StatusOptions::new();
    status_options.include_untracked(true);
    Ok(!repo.statuses(Some(&mut status_options))?.is_empty())
}

/// Whether a branch has commits that are neither merged into trunk nor
/// pushed to origin. A missing branch has nothing to lose.
pub fn branch_needs_push(bare_repo_path: &Path, branch: &str) -> Result<bool> {
    let repo = Repository::open(bare_repo_path)?;
    let tip = match repo.find_branch(branch, BranchType::Local) {
        Ok(b) => b.get().peel_to_commit()?.id(),
        Err(_) => return Ok(false),
    };

    let trunk = trunk_commit(&repo)?;
    if tip == trunk || repo.graph_descendant_of(trunk, tip)? {
        return Ok(false); // merged into trunk
    }
    if let Ok(remote) = repo.find_reference(&format!("refs/remotes/origin/{}", branch)) {
        let remote_tip = remote.peel_to_commit()?.id();
        if remote_tip == tip || repo.graph_descendant_of(remote_tip, tip)? {
            return Ok(false); // fully pushed
        }
    }
    Ok(true)
}

/// Re-points a detached view worktree at a commit with a safe checkout.
/// Refuses if the worktree has local changes — views are surfaced, never
/// destroyed.
pub fn retarget_worktree(worktree_path: &Path, at: Oid) -> Result<()> {
    let repo = Repository::open(worktree_path)?;

    let mut status_options = StatusOptions::new();
    status_options.include_untracked(true);
    if !repo.statuses(Some(&mut status_options))?.is_empty() {
        return Err(anyhow!(
            "{:?} has local changes — commit, stash, or discard them first",
            worktree_path
        ));
    }

    // Already there: nothing to do.
    if let Ok(head) = repo.head()
        && head.peel_to_commit().map(|c| c.id()).ok() == Some(at)
    {
        return Ok(());
    }

    let commit = repo.find_commit(at)?;
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.safe();
    repo.checkout_tree(commit.as_object(), Some(&mut checkout))?;
    repo.set_head_detached(at)?;
    Ok(())
}

/// Errors unless a branch is merged into trunk or fully pushed to origin —
/// the `git branch -d` safety equivalent for branch sets.
pub fn ensure_branch_disposable(bare_repo_path: &Path, branch: &str) -> Result<()> {
    if branch_needs_push(bare_repo_path, branch)? {
        return Err(anyhow!(
            "branch '{}' has commits that are neither merged into trunk nor pushed — \
             push them or use --force to discard",
            branch
        ));
    }
    Ok(())
}

/// Deletes a local branch from a cached bare repo. Missing branches are not
/// an error.
pub fn delete_branch(bare_repo_path: &Path, branch: &str) -> Result<()> {
    let repo = Repository::open(bare_repo_path)?;
    match repo.find_branch(branch, BranchType::Local) {
        Ok(mut b) => Ok(b.delete()?),
        Err(_) => Ok(()),
    }
}

/// Resolves the bare repository that owns a linked worktree. This beats
/// re-deriving the cache path from a recorded identity: it stays correct
/// even if the cache layout changes. Returns None if the path is not a
/// linked worktree (e.g. its bare repo was deleted).
pub fn owning_bare_repo(worktree_path: &Path) -> Option<PathBuf> {
    let repo = Repository::open(worktree_path).ok()?;
    if !repo.is_worktree() {
        return None;
    }
    Some(repo.commondir().to_path_buf())
}
