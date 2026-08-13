# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build
cargo test
cargo test branch_set_lifecycle        # single test by name
cargo test --test integration          # just the integration suite
cargo fmt -- --check                   # CI fails on unformatted code
cargo clippy -- -D warnings            # CI treats every warning as an error
cargo run -- status --json             # run the CLI
```

CI (`.github/workflows/ci.yml`) runs exactly build → test → fmt check → clippy on every push/PR to `main`. Rust edition is 2024 (`if let` chains are used throughout).

## Big picture

`myspace` manages multi-repo workspaces ("spaces") as three layers of git worktrees over one shared cache:

1. **Cache** — one bare clone per repo per machine at `~/.myspace/repos/<host>/<org>/<repo>` (`$MYSPACE_HOME` overrides the root). It fetches into **remote-tracking refs only** (`+refs/heads/*:refs/remotes/origin/*`); local heads are reserved for branch sets. This is a hard invariant — a fetch must never move a branch that a worktree has checked out, and `clone_bare` even deletes the local default branch libgit2 creates at clone time (leaving HEAD symbolic/unborn, which is how `trunk_commit` learns the default branch name).
2. **Canonical views** — `<space>/<repo>` worktrees, **always detached** (at trunk, or at a branch set's tips when a view is active). Detached is what lets N spaces hold the same repo and lets views look at a branch without claiming it.
3. **Branch sets** — `.myspace/trees/<set>/<repo>` worktrees, each **owning** the branch `<space>/<set>`. Git's one-branch-one-worktree rule is the concurrency control; myspace adds attribution (`branch_holder`) so the refusal names the holding worktree.

There is deliberately no database: git's own worktree registrations are the source of truth for who holds what; the only stored state is the manifest (`.myspace/config.toml`, shareable definition: space name + `repositories` map of name → identity), local view state (`.myspace/state.toml`), and a self-pruning advisory registry of space roots (`~/.myspace/spaces.toml`) used by `--all`.

## Layout and layering

Library + thin binary: `src/main.rs` only dispatches clap commands into `src/ops/mod.rs`, where every command is a function taking `ops::Context` (`cwd`, `config_path`, `cache_dir`, `registry_path`). That injection is what makes `tests/integration.rs` possible — it drives the real command implementations against temp dirs, building fixture repos with git2 (no `git` binary anywhere). Never reach for `env::current_dir()` or real paths inside `ops`; the one exception is `delete`'s chdir-out-of-the-workspace logic, which is genuinely about the live process.

- `engine/target_parser.rs` — target grammar + resolution (below), and `is_valid_component`, the **security boundary**: every name that ends up joined into a path (targets, manifest keys, identities, branch/set names) must pass it. It also bans `@` (the ref separator).
- `git/engine.rs` — all git work, via libgit2 only (**never shell out to git**; auth = SSH agent or per-host `ssh_key` from config — libgit2 does not read `~/.ssh/config`). Worktree quirks live here: creation detaches via a scratch-branch dance (libgit2 has no detach flag), registrations are name-deduped (`free_worktree_name`) and stale ones pruned, removal is `prune` with `valid+working_tree` flags plus a manual dirty check. `owning_bare_repo` maps a worktree back to its bare repo via `commondir` — never re-derive cache paths from URLs.
- `models/` — config (missing config is a **hard error** with setup guidance, never a default), manifest (`BTreeMap` so toml serializes deterministically), state, parsed target.
- `utils/` — paths (`~/.myspace`), per-repo file locks (`std::fs::File::lock` on a sibling `<repo>.lock`; hold one around any cache mutation), space registry, workspace-root discovery (walks up looking for `.myspace/config.toml`).

## Target notation

Three forms, each accepting an `@ref` suffix; parsing in `parse_target`, defaults filled by `resolve`:

```
mytool                            # bare → default_host + default_org
//gh/apache:impala                # bazel short; host slot = alias (no dot) or literal host (dot)
//gh:mytool                       # alias + default org
git@github.com:apache/impala.git  # raw scp URL accepted verbatim
```

Resolution produces `ResolvedRepo` whose `identity()` (`host/org/repo`) is what manifests record (portable across machines) and what the cache path is, verbatim. `.git` belongs to URLs only. SSH scp-style transport only; https and non-GitHub-shaped hosts (nested GitLab groups etc.) are documented expansion points, not supported.

## Invariants to preserve

- The cache has **zero local branches** except those created for branch sets (tested in `add_provisions_worktree_from_shared_cache`).
- Views are **surfaced, never destroyed**: `retarget_worktree` refuses dirty worktrees; sync/view report and skip them.
- Teardown safety: `branch -d` requires merged-into-trunk or pushed-to-origin (`branch_needs_push`) unless forced; `remove` refuses dirty worktrees unless forced.
- `add` rolls back the worktree if the manifest save fails; `remove` reconciles hand-deleted worktrees by dropping the manifest entry.
- The managed `.gitignore` block (markers in `ops`) is regenerated on membership changes and must preserve user content outside the markers.
- Nothing writes to stdout/stderr during git operations — console output is `println!` and the indicatif spinner only (libgit2 makes this automatic; don't reintroduce subprocesses).
- Examples in docs/comments use neutral names (`apache/impala`, `acme`, `myorg`) — never real internal project names.

## Testing

Unit tests: `#[cfg(test)]` in `engine/target_parser.rs`. Everything else: `tests/integration.rs` end-to-end against tempdirs (canonicalized — macOS `/var` symlink). git2 0.21 API sharp edges encountered here: `StringArray::iter` yields `Result<Option<&str>>` (double flatten), `Reference::name`/`symbolic_target` return `Result`, `Branch::delete` refuses HEAD's branch (delete the raw ref instead).
