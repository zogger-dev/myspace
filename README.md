# myspace

`myspace` is a developer utility for managing multi-repository Git workspaces ("spaces") using Git worktrees. A space bundles disparate git repositories into one directory your IDE can open, backed by bare clones cached once per machine — so provisioning a repo into a space, spinning up a branch across five repos, or tearing it all down again costs milliseconds, not clones.

## Features

*   **Zero-Overhead Provisioning:** One bare clone per repo per machine (`~/.myspace/repos/<host>/<org>/<repo>`); every checkout anywhere is a lightweight worktree of it.
*   **Bazel-Style Targets:** `repo`, `//alias{/org}:repo`, or a raw git URL — host aliases and config-driven defaults fill in the rest.
*   **Branch Sets:** One command creates a namespaced branch plus worktrees across every repo in the space — the unit of work for a feature or an agent.
*   **Switchable Views:** The space's canonical checkouts sit at trunk by default and can be pointed at any branch set for review, without stealing the branch from whoever is working on it.
*   **Context Aware:** Commands detect the enclosing space from your working directory.
*   **Self-Contained:** All git operations go through libgit2 — no `git` binary is required at runtime. SSH authentication uses your SSH agent.
*   **Cross Platform:** Linux, macOS, and Windows, with release binaries currently published for Linux and macOS.

## Installation

You can download pre-compiled binaries from the GitHub Releases page. Alternatively, if you have Rust installed, you can build it from source:

```bash
cargo install --path .
```

## Configuration

All global state lives under `~/.myspace` (override with `$MYSPACE_HOME`): `config.toml`, the bare-repo cache in `repos/<host>/<org>/<repo>`, a registry of your spaces in `spaces.toml`, and logs in `logs/`.

Example `~/.myspace/config.toml`:

```toml
default_org = "zogger-dev"
# default_host = "github.com"   # optional; this is the default

[aliases]
gh = "github.com"               # aliases map to bare hostnames
bb = "bitbucket.org"

[hosts."github.com"]
# Optional explicit SSH identity (the IdentityFile+IdentitiesOnly
# equivalent). libgit2 does not read ~/.ssh/config, and agent key order
# decides identity otherwise — set this if you have multiple keys for
# the same host.
# ssh_key = "~/.ssh/work_ed25519"
```

The config file is required for `myspace add` — without it there is no way to resolve targets into clone URLs, and `myspace` will tell you what to create rather than guessing.

## Anatomy of a space

```
my-space/
├── .gitignore          # managed block, safe to git init the space itself
├── .myspace/
│   ├── config.toml     # the manifest: space name + member repos (shareable)
│   ├── state.toml      # which view this machine is on (local, gitignored)
│   └── trees/
│       └── my-feature/ # a branch set: one worktree per repo, on branch my-space/my-feature
│           ├── repo-a/
│           └── repo-b/
├── repo-a/             # canonical views: detached at trunk (or the active view's tips)
└── repo-b/
```

The space root contains only the canonical repo views — that's what you open in your IDE. Everything else hides in `.myspace/`. The managed `.gitignore` block keeps member repos and local state out of source control, so you can version the space definition itself if you like. (Tip: VS Code's `explorer.excludeGitIgnore` setting hides ignored paths from the file explorer.)

## Usage

### Space lifecycle

*   `myspace init` — bootstrap the current directory as an empty space.
*   `myspace create my-space` — provision a new space directory.
*   `myspace edit [--ignore]` — open the manifest (or the space `.gitignore`) in `$EDITOR`; the manifest is validated after editing.
*   `myspace delete [name]` — tear down all worktrees and delete the space. The shared cache is untouched.

### Members

*   `myspace add <target>` — resolve, clone/reuse the cache, and provision a trunk view.
*   `myspace remove <name> [--force]` — tear down one worktree and drop it from the manifest; refuses to discard uncommitted changes without `--force`.

Targets come in three forms — bare, Bazel-style shorts, and raw git URLs. (An `@ref` suffix is parsed on any form but reserved for pinned dependency checkouts, which are not implemented yet — `add` rejects it for now.)

*   `myspace add mytool` — `git@github.com:zogger-dev/mytool.git` (default host + org).
*   `myspace add //gh/apache:impala` — `git@github.com:apache/impala.git` (alias `gh` in the host slot).
*   `myspace add //gh:mytool` — alias host, default org.
*   `myspace add //ghe.example.com/team:tool` — a literal host works in the slot too (a dot marks a host, no dot marks an alias).
*   `myspace add git@github.com:apache/impala.git` — raw URLs paste straight from a clone button.

### Branch sets

A branch set is where work happens: a git branch named `<space>/<name>` in every participating repo, each checked out in its own worktree under `.myspace/trees/<name>/`. The space namespace means the same set name in two different spaces never collides.

*   `myspace branch my-feature` — create the set across **all** members (worktrees are cheap); list specific repos to restrict it, re-run to grow it.
*   `myspace branch stacked --from my-feature` — stack a set on another instead of trunk.
*   `myspace branch -d my-feature` — tear the set down. Refuses if a worktree is dirty or the branch has commits neither merged into trunk nor pushed; `--force` overrides, `--keep-branch` keeps the branch refs.

Because branch-set worktrees *own* their branch, git itself guarantees nobody else — no other space, no raw git invocation — can check it out simultaneously; myspace turns that refusal into an error naming who holds it.

### Views and syncing

*   `myspace view my-feature` — re-point the canonical views (detached) at the set's tips, so your IDE shows the feature's code; repos not in the set stay at trunk. `myspace view trunk` switches back. Dirty views are reported, never overwritten.
*   `myspace sync [--all]` — fetch every cached repo of the space (or of all registered spaces) and refresh views at their current target.
*   `myspace status [--all] [--json]` — members, branch sets, active view, dirty/needs-push state; `--json` is the machine-readable form intended for agents.

### A typical flow

```bash
myspace create acme && cd acme
myspace add service-api
myspace add service-web
myspace branch fix-login          # branch + worktrees across both repos
# agent (or you) works in .myspace/trees/fix-login/…, commits, pushes
myspace view fix-login            # inspect the feature from your IDE
myspace view trunk
myspace branch -d fix-login       # after the PRs land
```

## License

This project is licensed under the MIT License.
