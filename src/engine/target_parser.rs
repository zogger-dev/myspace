use crate::models::config::GlobalConfig;
use crate::models::target::ParsedTarget;
use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};

/// Returns whether a target/name component is safe to use as a single path
/// segment. Rejects traversal (`..`), separators, `@` (the ref separator),
/// and control characters — components end up joined into worktree and
/// cache paths.
pub fn is_valid_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains(['/', '\\', ':', '@'])
        && !value.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn validate_component(value: &str, what: &str) -> Result<()> {
    if is_valid_component(value) {
        Ok(())
    } else {
        Err(anyhow!("Invalid {} '{}' in target", what, value))
    }
}

fn validate_ref(value: &str) -> Result<()> {
    if value.is_empty()
        || value.starts_with('-')
        || value.contains('\\')
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(anyhow!("Invalid ref '{}' in target", value));
    }
    Ok(())
}

/// Splits a trailing `@ref` pin off a string. Components may not contain
/// `@`, so the last `@` is always the separator.
fn split_ref(s: &str) -> Result<(&str, Option<String>)> {
    match s.rsplit_once('@') {
        Some((path, r)) => {
            validate_ref(r)?;
            Ok((path, Some(r.to_string())))
        }
        None => Ok((s, None)),
    }
}

/// Parses a target. Three forms (each takes an `@ref` suffix):
///
/// - `repo` — bare, default host and org
/// - `//hostish{/org}:repo` — Bazel-style short; hostish is an alias or a
///   literal host, org defaults when omitted
/// - `git@host:org/repo.git` — a raw scp-style URL, pasteable verbatim
pub fn parse_target(s: &str) -> Result<ParsedTarget> {
    if let Some(rest) = s.strip_prefix("//") {
        let (location, repo_part) = rest.split_once(':').ok_or_else(|| {
            anyhow!(
                "Invalid target '{}'. Expected //hostish:repo or //hostish/org:repo",
                s
            )
        })?;
        let (repo, git_ref) = split_ref(repo_part)?;
        let segments: Vec<&str> = location.split('/').collect();
        let (host, org) = match segments.len() {
            1 => (segments[0], None),
            2 => (segments[0], Some(segments[1])),
            _ => {
                return Err(anyhow!(
                    "Invalid target '{}'. Expected //hostish:repo or //hostish/org:repo",
                    s
                ));
            }
        };
        validate_component(host, "host")?;
        if let Some(org) = org {
            validate_component(org, "org")?;
        }
        validate_component(repo, "repo")?;
        Ok(ParsedTarget {
            host: Some(host.to_string()),
            org: org.map(str::to_string),
            repo: repo.to_string(),
            git_ref,
        })
    } else if let Some((userhost, path)) = s.split_once(':') {
        // Raw scp-style URL: git@host:org/repo(.git)(@ref)
        let host = userhost.rsplit_once('@').map_or(userhost, |(_, h)| h);
        let (path, git_ref) = split_ref(path)?;
        let path = path.strip_suffix(".git").unwrap_or(path);
        let (org, repo) = path
            .split_once('/')
            .ok_or_else(|| anyhow!("Invalid git URL '{}'. Expected git@host:org/repo.git", s))?;
        validate_component(host, "host")?;
        validate_component(org, "org")?;
        validate_component(repo, "repo")?;
        Ok(ParsedTarget {
            host: Some(host.to_string()),
            org: Some(org.to_string()),
            repo: repo.to_string(),
            git_ref,
        })
    } else {
        let (repo, git_ref) = split_ref(s)?;
        if repo.contains('/') {
            return Err(anyhow!(
                "Invalid target '{}'. Expected repo, //hostish{{/org}}:repo, or git@host:org/repo.git",
                s
            ));
        }
        validate_component(repo, "repo")?;
        Ok(ParsedTarget {
            host: None,
            org: None,
            repo: repo.to_string(),
            git_ref,
        })
    }
}

/// A target resolved against the global config: the clone URL plus the
/// host/org/repo coordinates. The identity string (`host/org/repo`) doubles
/// as the cache path and the src/ path — one dialect everywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRepo {
    pub url: String,
    pub host: String,
    pub org: String,
    pub repo: String,
    pub git_ref: Option<String>,
}

impl ResolvedRepo {
    /// The canonical identity string, `host/org/repo`. Recorded in manifests
    /// instead of URLs so a space definition is portable across machines
    /// whose transport differs.
    pub fn identity(&self) -> String {
        format!("{}/{}/{}", self.host, self.org, self.repo)
    }

    /// Bare repos are cached at `<cache>/<host>/<org>/<repo>` — the identity
    /// verbatim.
    pub fn cache_path(&self, cache_dir: &Path) -> PathBuf {
        cache_dir.join(&self.host).join(&self.org).join(&self.repo)
    }

    fn validate(&self) -> Result<()> {
        validate_host(&self.host)?;
        validate_component(&self.org, "org")?;
        validate_component(&self.repo, "repo")
    }
}

fn validate_host(host: &str) -> Result<()> {
    if host.contains('@') || host.contains(':') {
        return Err(anyhow!(
            "'{}' looks like a transport string. Hosts and alias values are bare \
             hostnames now, e.g. \"bitbucket.org\" — the git@/ssh part is derived",
            host
        ));
    }
    if !host.contains('.') {
        return Err(anyhow!("'{}' is not a valid host (no dot)", host));
    }
    validate_component(host, "host")
}

/// Resolves a ParsedTarget into a clone URL and cache coordinates.
/// A 3-segment first component with a dot is a literal host; without a dot
/// it is looked up in the config's aliases.
pub fn resolve(target: &ParsedTarget, config: &GlobalConfig) -> Result<ResolvedRepo> {
    let host = match &target.host {
        None => config.default_host.clone(),
        Some(h) if h.contains('.') => h.clone(),
        Some(alias) => config.aliases.get(alias).cloned().ok_or_else(|| {
            anyhow!(
                "'{}' has no dot, so it is treated as an alias — but no alias \
                 '{}' is configured",
                alias,
                alias
            )
        })?,
    };
    let org = target
        .org
        .clone()
        .unwrap_or_else(|| config.default_org.clone());

    let resolved = ResolvedRepo {
        // SSH scp-style only for now; `.git` belongs to the URL, never to
        // the identity. https/other transports are a documented expansion
        // point awaiting a real request.
        url: format!("git@{}:{}/{}.git", host, org, target.repo),
        host,
        org,
        repo: target.repo.clone(),
        git_ref: target.git_ref.clone(),
    };
    resolved.validate()?;
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn get_test_config() -> GlobalConfig {
        let mut aliases = HashMap::new();
        aliases.insert("gh".to_string(), "github.com".to_string());
        aliases.insert("bb".to_string(), "bitbucket.org".to_string());
        aliases.insert("legacy".to_string(), "git@bitbucket.org".to_string());

        GlobalConfig {
            default_host: "github.com".to_string(),
            default_org: "zogger-dev".to_string(),
            aliases,
            hosts: HashMap::new(),
        }
    }

    fn parsed(s: &str) -> ParsedTarget {
        parse_target(s).unwrap()
    }

    #[test]
    fn test_parse_bare() {
        assert_eq!(
            parsed("mytool"),
            ParsedTarget {
                host: None,
                org: None,
                repo: "mytool".to_string(),
                git_ref: None
            }
        );
    }

    #[test]
    fn test_parse_bazel_shorts() {
        assert_eq!(
            parsed("//gh/apache:impala"),
            ParsedTarget {
                host: Some("gh".to_string()),
                org: Some("apache".to_string()),
                repo: "impala".to_string(),
                git_ref: None
            }
        );
        assert_eq!(
            parsed("//gh:mytool"),
            ParsedTarget {
                host: Some("gh".to_string()),
                org: None,
                repo: "mytool".to_string(),
                git_ref: None
            }
        );
        // Literal hosts work in the host slot too.
        assert_eq!(
            parsed("//ghe.example.com/team:tool").host.as_deref(),
            Some("ghe.example.com")
        );
    }

    #[test]
    fn test_parse_raw_url() {
        let t = parsed("git@github.com:apache/impala.git");
        assert_eq!(t.host.as_deref(), Some("github.com"));
        assert_eq!(t.org.as_deref(), Some("apache"));
        assert_eq!(t.repo, "impala");
        // .git suffix is optional.
        assert_eq!(parsed("git@github.com:apache/impala").repo, "impala");
    }

    #[test]
    fn test_parse_ref_suffix() {
        assert_eq!(parsed("mytool@v1.2.3").git_ref.as_deref(), Some("v1.2.3"));
        assert_eq!(
            parsed("//gh/apache:impala@abc123").git_ref.as_deref(),
            Some("abc123")
        );
        assert_eq!(
            parsed("git@github.com:apache/impala.git@v2")
                .git_ref
                .as_deref(),
            Some("v2")
        );
        // Branch-like refs may contain slashes.
        assert_eq!(
            parsed("mytool@releases/v2").git_ref.as_deref(),
            Some("releases/v2")
        );
    }

    #[test]
    fn test_parse_rejects_invalid() {
        assert!(parse_target("").is_err());
        assert!(parse_target("..").is_err());
        assert!(parse_target(".").is_err());
        assert!(parse_target("apache/impala").is_err()); // old slash form
        assert!(parse_target("../repo").is_err());
        assert!(parse_target("//gh/a/b:repo").is_err());
        assert!(parse_target("//gh/apache").is_err()); // missing :repo
        assert!(parse_target("//gh:..").is_err());
        assert!(parse_target("//gh/..:repo").is_err());
        assert!(parse_target("git@github.com:impala.git").is_err()); // no org
        assert!(parse_target("re po").is_err());
        assert!(parse_target("repo@").is_err());
        assert!(parse_target("@v1").is_err());
        assert!(parse_target("repo@-bad").is_err());
    }

    #[test]
    fn test_resolve_defaults() {
        let config = get_test_config();
        let resolved = resolve(&parsed("mytool"), &config).unwrap();
        assert_eq!(resolved.url, "git@github.com:zogger-dev/mytool.git");
        assert_eq!(resolved.identity(), "github.com/zogger-dev/mytool");
    }

    #[test]
    fn test_resolve_alias_short() {
        let config = get_test_config();
        let resolved = resolve(&parsed("//bb/team:tool"), &config).unwrap();
        assert_eq!(resolved.url, "git@bitbucket.org:team/tool.git");
        assert_eq!(resolved.host, "bitbucket.org");

        // Alias with default org.
        let resolved = resolve(&parsed("//bb:tool"), &config).unwrap();
        assert_eq!(resolved.url, "git@bitbucket.org:zogger-dev/tool.git");
    }

    #[test]
    fn test_resolve_literal_host_short() {
        let config = get_test_config();
        let resolved = resolve(&parsed("//ghe.example.com/team:tool"), &config).unwrap();
        assert_eq!(resolved.url, "git@ghe.example.com:team/tool.git");
    }

    #[test]
    fn test_resolve_raw_url_roundtrip() {
        let config = get_test_config();
        let resolved = resolve(&parsed("git@github.com:apache/impala.git"), &config).unwrap();
        assert_eq!(resolved.url, "git@github.com:apache/impala.git");
        assert_eq!(resolved.identity(), "github.com/apache/impala");
    }

    #[test]
    fn test_resolve_unknown_alias() {
        let config = get_test_config();
        let err = resolve(&parsed("//nope/team:tool"), &config).unwrap_err();
        assert!(err.to_string().contains("alias"), "{}", err);
    }

    #[test]
    fn test_resolve_rejects_transport_style_alias_value() {
        let config = get_test_config();
        let err = resolve(&parsed("//legacy/team:tool"), &config).unwrap_err();
        assert!(err.to_string().contains("bare hostnames"), "{}", err);
    }

    #[test]
    fn test_resolve_carries_ref() {
        let config = get_test_config();
        let resolved = resolve(&parsed("mytool@v1.2.3"), &config).unwrap();
        assert_eq!(resolved.git_ref.as_deref(), Some("v1.2.3"));
        // The ref never leaks into url or identity.
        assert_eq!(resolved.url, "git@github.com:zogger-dev/mytool.git");
        assert_eq!(resolved.identity(), "github.com/zogger-dev/mytool");
    }

    #[test]
    fn test_cache_path_is_identity() {
        let config = get_test_config();
        let resolved = resolve(&parsed("//gh/apache:impala"), &config).unwrap();
        assert_eq!(
            resolved.cache_path(Path::new("/cache")),
            Path::new("/cache/github.com/apache/impala")
        );
    }
}
