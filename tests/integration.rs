use myspace::engine::target_parser::ResolvedRepo;
use myspace::models::config::GlobalConfig;
use myspace::models::manifest::WorkspaceManifest;
use myspace::ops::{self, Context};
use std::fs;
use std::path::{Path, PathBuf};

struct TestEnv {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

fn test_env() -> TestEnv {
    let tmp = tempfile::tempdir().expect("create tempdir");
    // Canonicalize so path comparisons survive macOS's /var -> /private/var symlink.
    let root = tmp.path().canonicalize().expect("canonicalize tempdir");
    TestEnv { _tmp: tmp, root }
}

fn ctx_at(env: &TestEnv, cwd: &Path) -> Context {
    Context {
        cwd: cwd.to_path_buf(),
        config_path: env.root.join("config.toml"),
        cache_dir: env.root.join("cache"),
        registry_path: env.root.join("spaces.toml"),
    }
}

/// Stages everything and commits (with parent) in the repo at `repo_path`.
fn commit_all(repo_path: &Path, message: &str) {
    let repo = git2::Repository::open(repo_path).unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
        .unwrap();
}

fn head_oid(repo_path: &Path) -> git2::Oid {
    git2::Repository::open(repo_path)
        .unwrap()
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
}

/// Creates a local git repo usable as a clone source, and the ResolvedRepo
/// pointing at it. Built with git2 — the test suite, like the tool, needs no
/// `git` binary.
fn make_remote_named(env: &TestEnv, repo_name: &str) -> ResolvedRepo {
    let src = env.root.join(format!("remote-{}", repo_name));
    let repo = git2::Repository::init(&src).unwrap();
    fs::write(src.join("README.md"), "hello").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("README.md")).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();

    ResolvedRepo {
        url: src.to_string_lossy().into_owned(),
        host: "local".to_string(),
        org: "testorg".to_string(),
        repo: repo_name.to_string(),
        git_ref: None,
    }
}

fn make_remote(env: &TestEnv) -> ResolvedRepo {
    make_remote_named(env, "myrepo")
}

fn make_workspace(env: &TestEnv, name: &str) -> (Context, PathBuf) {
    let ctx_root = ctx_at(env, &env.root);
    ops::create(&ctx_root, name).unwrap();
    let ws = env.root.join(name);
    (ctx_at(env, &ws), ws)
}

/// The rest of the suite clones from local filesystem paths, which use
/// libgit2's always-present `local` transport — so no other test can catch a
/// libgit2 built without SSH. That is exactly how a real regression shipped
/// (git2 0.21 changed `default = []`, dropping the ssh feature), making every
/// `git@host:…` clone fail with "unsupported URL protocol; class=Net".
#[test]
fn libgit2_has_ssh_transport_compiled_in() {
    assert!(
        git2::Version::get().ssh(),
        "libgit2 was built without SSH support — check git2's `ssh` feature in Cargo.toml"
    );
}

#[test]
fn init_creates_manifest_and_refuses_twice() {
    let env = test_env();
    let ws = env.root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let ctx = ctx_at(&env, &ws);

    ops::init(&ctx).unwrap();
    let manifest = WorkspaceManifest::load(&ws.join(".myspace/config.toml")).unwrap();
    assert_eq!(manifest.workspace.name, "ws");
    assert!(manifest.repositories.is_empty());

    assert!(ops::init(&ctx).is_err());
}

#[test]
fn create_provisions_workspace_and_rejects_escapes() {
    let env = test_env();
    let ctx = ctx_at(&env, &env.root);

    ops::create(&ctx, "ws").unwrap();
    assert!(env.root.join("ws/.myspace/config.toml").exists());
    assert!(env.root.join("ws/.gitignore").exists());

    assert!(ops::create(&ctx, "ws").is_err()); // already exists
    assert!(ops::create(&ctx, "../escape").is_err());
    assert!(ops::create(&ctx, "/abs/path").is_err());
    // A space name is one component — it doubles as the branch namespace,
    // and multi-segment names could traverse through symlinks.
    assert!(ops::create(&ctx, "nested/space").is_err());
    assert!(ops::create(&ctx, "bad name").is_err());
}

#[test]
fn init_rejects_unusable_directory_names() {
    let env = test_env();
    let bad = env.root.join("bad name");
    fs::create_dir_all(&bad).unwrap();
    let err = ops::init(&ctx_at(&env, &bad)).unwrap_err();
    assert!(err.to_string().contains("branch namespace"), "{}", err);
}

#[test]
fn add_provisions_worktree_from_shared_cache() {
    let env = test_env();
    let resolved = make_remote(&env);
    let (ctx, ws) = make_workspace(&env, "ws");

    ops::add_resolved(&ctx, &ws, &resolved, None, None).unwrap();

    // Worktree is checked out with content.
    assert_eq!(
        fs::read_to_string(ws.join("myrepo/README.md")).unwrap(),
        "hello"
    );
    // Bare repo is cached at <cache>/<host>/<org>/<repo>.
    let cache = env.root.join("cache/local/testorg/myrepo");
    assert!(cache.join("HEAD").exists());
    // The cache is remote-tracking: no local heads (those are reserved for
    // branch sets), trunk lives under refs/remotes/origin/.
    let cache_repo = git2::Repository::open_bare(&cache).unwrap();
    let local_branches: Vec<_> = cache_repo
        .branches(Some(git2::BranchType::Local))
        .unwrap()
        .filter_map(|b| b.ok())
        .collect();
    assert!(
        local_branches.is_empty(),
        "cache must have no local branches"
    );
    assert!(
        cache_repo
            .branches(Some(git2::BranchType::Remote))
            .unwrap()
            .next()
            .is_some(),
        "cache must have remote-tracking branches"
    );
    // Manifest records the identity (host/org/repo), not the URL.
    let manifest = WorkspaceManifest::load(&ws.join(".myspace/config.toml")).unwrap();
    assert_eq!(
        manifest.repositories.get("myrepo"),
        Some(&resolved.identity())
    );

    // Re-adding the same name fails cleanly.
    assert!(ops::add_resolved(&ctx, &ws, &resolved, None, None).is_err());

    // A failed worktree creation must not leak its scratch branch into the
    // cache (local heads are reserved for branch sets).
    let blocked = ws.join("blocked");
    fs::write(&blocked, "a file, not a directory").unwrap();
    assert!(myspace::git::engine::add_worktree(&cache, &blocked, None).is_err());
    assert_eq!(
        cache_repo
            .branches(Some(git2::BranchType::Local))
            .unwrap()
            .count(),
        0,
        "scratch branch leaked from failed worktree creation"
    );

    // A second workspace reuses the cache (fetch path) rather than recloning.
    let (ctx2, ws2) = make_workspace(&env, "ws2");
    ops::add_resolved(&ctx2, &ws2, &resolved, None, None).unwrap();
    assert!(ws2.join("myrepo/README.md").exists());
}

#[test]
fn add_rejects_same_name_different_identity() {
    let env = test_env();
    let resolved = make_remote(&env);
    let (ctx, ws) = make_workspace(&env, "ws");
    ops::add_resolved(&ctx, &ws, &resolved, None, None).unwrap();

    let other = ResolvedRepo {
        org: "otherorg".to_string(),
        ..resolved.clone()
    };
    let err = ops::add_resolved(&ctx, &ws, &other, None, None).unwrap_err();
    assert!(err.to_string().contains("already tracks"), "{}", err);
}

#[test]
fn add_rejects_ref_pins_for_now() {
    let env = test_env();
    let resolved = make_remote(&env);
    let (ctx, ws) = make_workspace(&env, "ws");

    let pinned = ResolvedRepo {
        git_ref: Some("v1.0.0".to_string()),
        ..resolved
    };
    let err = ops::add_resolved(&ctx, &ws, &pinned, None, None).unwrap_err();
    // Must explain the situation without pointing at CLI options that don't
    // exist yet.
    assert!(err.to_string().contains("not usable"), "{}", err);
    assert!(!err.to_string().contains("--dep"), "{}", err);
}

#[test]
fn managed_gitignore_tracks_members_and_preserves_user_content() {
    let env = test_env();
    let resolved = make_remote(&env);
    let (ctx, ws) = make_workspace(&env, "ws");
    let gitignore = ws.join(".gitignore");

    // Created with the space: local state and machinery are ignored.
    let content = fs::read_to_string(&gitignore).unwrap();
    assert!(content.contains(".myspace/state.toml"), "{}", content);
    assert!(content.contains(".myspace/trees/"), "{}", content);

    // User content outside the managed block survives regeneration.
    fs::write(&gitignore, format!("# mine\ncustom.txt\n\n{}", content)).unwrap();

    ops::add_resolved(&ctx, &ws, &resolved, None, None).unwrap();
    let content = fs::read_to_string(&gitignore).unwrap();
    assert!(content.contains("/myrepo/"), "{}", content);
    assert!(content.contains("custom.txt"), "{}", content);
    assert_eq!(
        content.matches(".myspace/trees/").count(),
        1,
        "managed block must not duplicate:\n{}",
        content
    );

    ops::remove(&ctx, "myrepo", false).unwrap();
    let content = fs::read_to_string(&gitignore).unwrap();
    assert!(!content.contains("/myrepo/"), "{}", content);
    assert!(content.contains("custom.txt"), "{}", content);
}

#[test]
fn branch_set_lifecycle() {
    let env = test_env();
    let repo_a = make_remote_named(&env, "repo-a");
    let repo_b = make_remote_named(&env, "repo-b");
    let (ctx, ws) = make_workspace(&env, "ws");
    ops::add_resolved(&ctx, &ws, &repo_a, None, None).unwrap();
    ops::add_resolved(&ctx, &ws, &repo_b, None, None).unwrap();

    // Create the set with one repo, on the namespaced branch ws/feat.
    ops::branch(&ctx, "feat", &["repo-a".to_string()], None).unwrap();
    let set_a = ws.join(".myspace/trees/feat/repo-a");
    assert!(set_a.join("README.md").exists());
    let wt_repo = git2::Repository::open(&set_a).unwrap();
    assert_eq!(
        wt_repo.head().unwrap().name().unwrap(),
        "refs/heads/ws/feat"
    );

    // Idempotent re-run, then grow with the second repo.
    ops::branch(&ctx, "feat", &["repo-a".to_string()], None).unwrap();
    ops::branch(&ctx, "feat", &["repo-b".to_string()], None).unwrap();
    assert!(ws.join(".myspace/trees/feat/repo-b/README.md").exists());

    // No repos given → the whole space joins.
    ops::branch(&ctx, "everything", &[], None).unwrap();
    assert!(
        ws.join(".myspace/trees/everything/repo-a/README.md")
            .exists()
    );
    assert!(
        ws.join(".myspace/trees/everything/repo-b/README.md")
            .exists()
    );
    ops::branch_delete(&ctx, "everything", false, false).unwrap();

    // Non-member repos are refused.
    let err = ops::branch(&ctx, "feat", &["ghost".to_string()], None).unwrap_err();
    assert!(err.to_string().contains("not a member"), "{}", err);

    // The namespaced branch exists in the shared cache, and git's
    // exclusivity rule holds: a second worktree claiming it is refused.
    let cache = env.root.join("cache/local/testorg/repo-a");
    let cache_repo = git2::Repository::open_bare(&cache).unwrap();
    assert!(
        cache_repo
            .find_branch("ws/feat", git2::BranchType::Local)
            .is_ok()
    );
    let tip = myspace::git::engine::branch_tip(&cache, "ws/feat").unwrap();
    let steal = env.root.join("steal");
    let err = myspace::git::engine::add_worktree_on_branch(&cache, &steal, "ws/feat", tip)
        .expect_err("checked-out branch must be unclaimable elsewhere");
    // Conflict attribution: the error names the holder, from git's records.
    assert!(
        err.to_string().contains("trees/feat/repo-a"),
        "error must name the holding worktree: {}",
        err
    );

    // Tear down: branch tip equals trunk (no commits) → deletable unforced.
    ops::branch_delete(&ctx, "feat", false, false).unwrap();
    assert!(!ws.join(".myspace/trees/feat").exists());
    assert!(
        cache_repo
            .find_branch("ws/feat", git2::BranchType::Local)
            .is_err(),
        "branch must be deleted with the set"
    );
}

#[test]
fn branch_delete_refusal_leaves_set_intact() {
    let env = test_env();
    let repo_a = make_remote_named(&env, "repo-a");
    let repo_b = make_remote_named(&env, "repo-b");
    let (ctx, ws) = make_workspace(&env, "ws");
    ops::add_resolved(&ctx, &ws, &repo_a, None, None).unwrap();
    ops::add_resolved(&ctx, &ws, &repo_b, None, None).unwrap();
    ops::branch(&ctx, "feat", &[], None).unwrap();

    // Dirty only the second (sorted) entry: preflight must catch it before
    // the first entry is destroyed.
    fs::write(ws.join(".myspace/trees/feat/repo-b/README.md"), "dirty").unwrap();
    let err = ops::branch_delete(&ctx, "feat", false, false).unwrap_err();
    assert!(err.to_string().contains("uncommitted"), "{}", err);
    assert!(
        ws.join(".myspace/trees/feat/repo-a/README.md").exists(),
        "refusal must not partially destroy the set"
    );
    assert!(ws.join(".myspace/trees/feat/repo-b/README.md").exists());
}

#[test]
fn delete_deregisters_branch_set_worktrees() {
    let env = test_env();
    let repo_a = make_remote_named(&env, "repo-a");
    let (ctx_ws, ws) = make_workspace(&env, "ws");
    ops::add_resolved(&ctx_ws, &ws, &repo_a, None, None).unwrap();
    ops::branch(&ctx_ws, "feat", &[], None).unwrap();

    let ctx_root = ctx_at(&env, &env.root);
    ops::delete(&ctx_root, Some("ws")).unwrap();
    assert!(!ws.exists());

    // No stale registrations may remain in the shared cache, and the branch
    // must survive teardown but no longer count as checked out.
    let cache = env.root.join("cache/local/testorg/repo-a");
    let cache_repo = git2::Repository::open_bare(&cache).unwrap();
    assert_eq!(cache_repo.worktrees().unwrap().iter().count(), 0);
    let mut branch = cache_repo
        .find_branch("ws/feat", git2::BranchType::Local)
        .expect("branch refs must be preserved by space deletion");
    branch
        .delete()
        .expect("branch must not appear checked out after deletion");
}

#[test]
fn branch_delete_refuses_unpushed_commits() {
    let env = test_env();
    let repo_a = make_remote_named(&env, "repo-a");
    let (ctx, ws) = make_workspace(&env, "ws");
    ops::add_resolved(&ctx, &ws, &repo_a, None, None).unwrap();
    ops::branch(&ctx, "feat", &["repo-a".to_string()], None).unwrap();

    // Commit on the branch — now it has work that is neither merged nor pushed.
    let set_a = ws.join(".myspace/trees/feat/repo-a");
    fs::write(set_a.join("new.txt"), "work").unwrap();
    commit_all(&set_a, "feature work");

    let err = ops::branch_delete(&ctx, "feat", false, false).unwrap_err();
    assert!(err.to_string().contains("neither merged"), "{}", err);
    assert!(set_a.exists(), "refused delete must not remove anything");

    // --keep-branch + force removes worktrees but keeps the branch ref.
    ops::branch_delete(&ctx, "feat", true, true).unwrap();
    assert!(!set_a.exists());
    let cache_repo =
        git2::Repository::open_bare(env.root.join("cache/local/testorg/repo-a")).unwrap();
    assert!(
        cache_repo
            .find_branch("ws/feat", git2::BranchType::Local)
            .is_ok(),
        "--keep-branch must preserve the branch"
    );
}

#[test]
fn view_switches_between_trunk_and_branch_set() {
    let env = test_env();
    let repo_a = make_remote_named(&env, "repo-a");
    let (ctx, ws) = make_workspace(&env, "ws");
    ops::add_resolved(&ctx, &ws, &repo_a, None, None).unwrap();
    ops::branch(&ctx, "feat", &["repo-a".to_string()], None).unwrap();

    // Agent commits in the branch-set worktree.
    let set_a = ws.join(".myspace/trees/feat/repo-a");
    fs::write(set_a.join("feature.txt"), "wip").unwrap();
    commit_all(&set_a, "feature work");
    let branch_tip = head_oid(&set_a);

    // Unknown sets are refused.
    assert!(ops::view(&ctx, "nope").is_err());

    // View the branch set: the canonical worktree shows the feature code,
    // detached at the branch tip (the branch stays owned by the set).
    ops::view(&ctx, "feat").unwrap();
    let view = ws.join("repo-a");
    assert!(view.join("feature.txt").exists());
    assert_eq!(head_oid(&view), branch_tip);
    let view_repo = git2::Repository::open(&view).unwrap();
    assert!(view_repo.head_detached().unwrap());

    // Back to trunk: feature file gone.
    ops::view(&ctx, "trunk").unwrap();
    assert!(!view.join("feature.txt").exists());

    // A dirty view is reported but never touched.
    fs::write(view.join("README.md"), "local edit").unwrap();
    ops::view(&ctx, "feat").unwrap(); // warning, not error
    assert_eq!(
        fs::read_to_string(view.join("README.md")).unwrap(),
        "local edit",
        "dirty view must not be clobbered"
    );
}

#[test]
fn sync_refreshes_trunk_views() {
    let env = test_env();
    let repo_a = make_remote_named(&env, "repo-a");
    let (ctx, ws) = make_workspace(&env, "ws");
    ops::add_resolved(&ctx, &ws, &repo_a, None, None).unwrap();

    // Trunk moves upstream after the initial clone.
    let remote_src = env.root.join("remote-repo-a");
    fs::write(remote_src.join("later.txt"), "new").unwrap();
    commit_all(&remote_src, "second commit");

    assert!(!ws.join("repo-a/later.txt").exists());
    ops::sync(&ctx, false).unwrap();
    assert!(
        ws.join("repo-a/later.txt").exists(),
        "view must follow trunk"
    );
}

#[test]
fn sync_all_uses_registry() {
    let env = test_env();
    let repo_a = make_remote_named(&env, "repo-a");
    let (ctx1, ws1) = make_workspace(&env, "ws1");
    let (_ctx2, ws2) = make_workspace(&env, "ws2");
    ops::add_resolved(&ctx1, &ws1, &repo_a, None, None).unwrap();
    let ctx2 = ctx_at(&env, &ws2);
    ops::add_resolved(&ctx2, &ws2, &repo_a, None, None).unwrap();

    let remote_src = env.root.join("remote-repo-a");
    fs::write(remote_src.join("later.txt"), "new").unwrap();
    commit_all(&remote_src, "second commit");

    // Run --all from a directory that is not a space at all.
    let outside = ctx_at(&env, &env.root);
    ops::sync(&outside, true).unwrap();
    assert!(ws1.join("repo-a/later.txt").exists());
    assert!(ws2.join("repo-a/later.txt").exists());

    // Deleting a space unregisters it: sync --all must not fail afterwards.
    ops::delete(&outside, Some("ws2")).unwrap();
    ops::sync(&outside, true).unwrap();
}

#[test]
fn status_reports_spaces_repos_and_branch_sets() {
    let env = test_env();
    let repo_a = make_remote_named(&env, "repo-a");
    let (ctx, ws) = make_workspace(&env, "ws");
    ops::add_resolved(&ctx, &ws, &repo_a, None, None).unwrap();
    ops::branch(&ctx, "feat", &["repo-a".to_string()], None).unwrap();

    let set_a = ws.join(".myspace/trees/feat/repo-a");
    fs::write(set_a.join("new.txt"), "wip").unwrap();
    commit_all(&set_a, "feature work");
    fs::write(set_a.join("scratch.txt"), "uncommitted").unwrap();
    ops::view(&ctx, "feat").unwrap();

    let status = ops::space_status(&ws).unwrap();
    assert_eq!(status.name, "ws");
    assert_eq!(status.view, "feat");
    assert_eq!(status.repos.len(), 1);
    assert!(status.repos[0].present);
    assert_eq!(status.repos[0].dirty, Some(false));
    assert_eq!(status.repos[0].identity, repo_a.identity());

    assert_eq!(status.branch_sets.len(), 1);
    let set = &status.branch_sets[0];
    assert_eq!(set.branch, "ws/feat");
    assert_eq!(set.repos.len(), 1);
    assert_eq!(set.repos[0].dirty, Some(true), "uncommitted scratch file");
    assert_eq!(set.repos[0].needs_push, Some(true), "local-only commit");

    // The JSON shape serializes.
    let json = serde_json::to_string(&status).unwrap();
    assert!(json.contains("\"view\":\"feat\""), "{}", json);
}

#[test]
fn config_parses_hosts_and_expands_ssh_key() {
    let env = test_env();
    let path = env.root.join("config.toml");
    fs::write(
        &path,
        r#"
default_org = "myorg"

[aliases]
bb = "bitbucket.org"

[hosts."github.com"]
ssh_key = "~/.ssh/work_ed25519"
"#,
    )
    .unwrap();
    let config = GlobalConfig::load(&path).unwrap();
    assert_eq!(config.default_host, "github.com"); // serde default
    let key = config.ssh_key_for("github.com").unwrap();
    assert!(key.is_absolute(), "tilde must expand: {:?}", key);
    assert!(key.ends_with(".ssh/work_ed25519"));
    assert_eq!(config.ssh_key_for("bitbucket.org"), None);
}

#[test]
fn remove_deletes_worktree_and_manifest_entry() {
    let env = test_env();
    let resolved = make_remote(&env);
    let (ctx, ws) = make_workspace(&env, "ws");
    ops::add_resolved(&ctx, &ws, &resolved, None, None).unwrap();

    ops::remove(&ctx, "myrepo", false).unwrap();

    assert!(!ws.join("myrepo").exists());
    let manifest = WorkspaceManifest::load(&ws.join(".myspace/config.toml")).unwrap();
    assert!(manifest.repositories.is_empty());
    // The shared cache is untouched.
    assert!(env.root.join("cache/local/testorg/myrepo/HEAD").exists());

    // Removing an untracked repo errors.
    assert!(ops::remove(&ctx, "myrepo", false).is_err());
    // Traversal names are rejected outright.
    assert!(ops::remove(&ctx, "..", false).is_err());
}

#[test]
fn remove_without_force_refuses_dirty_worktree() {
    let env = test_env();
    let resolved = make_remote(&env);
    let (ctx, ws) = make_workspace(&env, "ws");
    ops::add_resolved(&ctx, &ws, &resolved, None, None).unwrap();

    fs::write(ws.join("myrepo/README.md"), "dirty").unwrap();
    let err = ops::remove(&ctx, "myrepo", false).unwrap_err();
    assert!(err.to_string().contains("uncommitted changes"), "{}", err);
    assert!(ws.join("myrepo").exists());

    ops::remove(&ctx, "myrepo", true).unwrap();
    assert!(!ws.join("myrepo").exists());
}

#[test]
fn remove_refuses_orphaned_worktree_without_force() {
    let env = test_env();
    let resolved = make_remote(&env);
    let (ctx, ws) = make_workspace(&env, "ws");
    ops::add_resolved(&ctx, &ws, &resolved, None, None).unwrap();

    // Cache gone → no dirty-check possible → unforced removal must refuse
    // rather than silently discard whatever is in the checkout.
    fs::remove_dir_all(env.root.join("cache/local/testorg/myrepo")).unwrap();
    let err = ops::remove(&ctx, "myrepo", false).unwrap_err();
    assert!(err.to_string().contains("--force"), "{}", err);
    assert!(ws.join("myrepo/README.md").exists());

    ops::remove(&ctx, "myrepo", true).unwrap();
    assert!(!ws.join("myrepo").exists());
}

#[test]
fn remove_reconciles_hand_deleted_worktree() {
    let env = test_env();
    let resolved = make_remote(&env);
    let (ctx, ws) = make_workspace(&env, "ws");
    ops::add_resolved(&ctx, &ws, &resolved, None, None).unwrap();

    fs::remove_dir_all(ws.join("myrepo")).unwrap();
    ops::remove(&ctx, "myrepo", false).unwrap();
    let manifest = WorkspaceManifest::load(&ws.join(".myspace/config.toml")).unwrap();
    assert!(manifest.repositories.is_empty());

    // The stale registration must not block re-adding.
    ops::add_resolved(&ctx, &ws, &resolved, None, None).unwrap();
    assert!(ws.join("myrepo/README.md").exists());
}

#[test]
fn delete_tears_down_workspace_but_keeps_cache() {
    let env = test_env();
    let resolved = make_remote(&env);
    let (ctx_ws, ws) = make_workspace(&env, "ws");
    ops::add_resolved(&ctx_ws, &ws, &resolved, None, None).unwrap();

    let ctx_root = ctx_at(&env, &env.root);
    ops::delete(&ctx_root, Some("ws")).unwrap();

    assert!(!ws.exists());
    assert!(env.root.join("cache/local/testorg/myrepo/HEAD").exists());
}

#[test]
fn missing_config_is_a_hard_error_with_guidance() {
    let env = test_env();
    let err = GlobalConfig::load(&env.root.join("config.toml")).unwrap_err();
    assert!(err.to_string().contains("No config found"), "{}", err);
    assert!(err.to_string().contains("default_org"), "{}", err);
}

#[test]
fn invalid_config_reports_path() {
    let env = test_env();
    let path = env.root.join("config.toml");
    fs::write(&path, "not [valid toml").unwrap();
    let err = GlobalConfig::load(&path).unwrap_err();
    assert!(err.to_string().contains("Invalid config"), "{}", err);
}

#[test]
fn manifest_serialization_is_deterministic() {
    let env = test_env();
    let (_, ws) = make_workspace(&env, "ws");
    let manifest_path = ws.join(".myspace/config.toml");

    let mut manifest = WorkspaceManifest::load(&manifest_path).unwrap();
    for name in ["zeta", "alpha", "mid"] {
        manifest
            .repositories
            .insert(name.to_string(), format!("git@github.com:o/{}.git", name));
    }
    manifest.save(&manifest_path).unwrap();
    let first = fs::read_to_string(&manifest_path).unwrap();

    let reloaded = WorkspaceManifest::load(&manifest_path).unwrap();
    reloaded.save(&manifest_path).unwrap();
    let second = fs::read_to_string(&manifest_path).unwrap();

    assert_eq!(first, second);
    let alpha = first.find("alpha").unwrap();
    let zeta = first.find("zeta").unwrap();
    assert!(alpha < zeta, "entries must be sorted:\n{}", first);
}
