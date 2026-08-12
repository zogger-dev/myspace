# myspace

`myspace` is a developer utility designed to manage multi-repository Git workspaces using Git worktrees. It enables developers to curate "workspaces" (defined by a `myspace.toml` manifest) that bundle together multiple, disparate git repositories efficiently. Under the hood, `myspace` leverages bare clones cached globally on your machine to provision blazing-fast, lightweight Git worktrees directly into your active workspace.

## Features

*   **Zero-Overhead Cloning:** Uses centralized bare repositories (stored in your OS's native data directory) to create ultra-fast local Git worktrees.
*   **Hierarchical Target Engine:** Smart parsing syntax allowing you to clone repositories using simple aliases, organization names, or full providers (`repo`, `org/repo`, `//alias/org/repo`).
*   **Context Aware:** Commands detect if you are inside an active workspace directory recursively, meaning you don't have to specify workspace names once you are working inside one.
*   **Cross Platform:** Native paths and configurations for Linux, macOS, and Windows, with release binaries currently published for Linux and macOS.

## Installation

You can download pre-compiled binaries from the GitHub Releases page. Alternatively, if you have Rust installed, you can build it from source:

```bash
cargo install --path .
```

## Configuration

`myspace` utilizes a global configuration file to define default SCM providers and custom aliases:

* Linux: `~/.config/myspace/config.toml`
* macOS: `~/Library/Application Support/com.myspace.myspace/config.toml`
* Windows: `%APPDATA%\myspace\myspace\config\config.toml`

Example `config.toml`:

```toml
default_org = "zogger-dev"
default_scm = "git@github.com"

[aliases]
bb = "git@bitbucket.org"
gl = "git@gitlab.com"
```

## Usage

### Workspace Lifecycle

*   **Initialize an existing directory:** `myspace init`
    Bootstraps an empty `myspace.toml` in the current directory.
*   **Create a new workspace:** `myspace create my-project`
    Provisions a new directory and manifest file.
*   **Edit the manifest:** `myspace edit`
    Opens `myspace.toml` in your `$EDITOR`.
*   **Delete the workspace:** `myspace delete`
    Safely tears down the worktrees and deletes the workspace directory. If run from inside the workspace, it shifts your execution context back to `$HOME`.

### Adding Repositories

Use `myspace add <target>` inside your workspace to clone and provision repositories.

*   `myspace add repo` - Expands using your `default_org` and `default_scm` (e.g., `git@github.com:zogger-dev/repo.git`).
*   `myspace add apache/impala` - Overrides the organization (e.g., `git@github.com:apache/impala.git`).
*   `myspace add //bb/myorg/myrepo` - Uses an alias to resolve custom hosts (e.g., `git@bitbucket.org:myorg/myrepo.git`).

## License

This project is licensed under the MIT License.
