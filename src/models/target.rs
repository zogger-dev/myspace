use std::fmt;

/// A parsed target string. Three forms:
///
/// - bare:  `repo` — default host and org
/// - short: `//hostish{/org}:repo` — Bazel-style; `hostish` is an alias
///   (`//gh/apache:impala`) or a literal host (`//ghe.example.com:tool`),
///   org defaults when omitted
/// - full:  a raw scp-style git URL, `git@host:org/repo.git`, pasteable
///   straight from a clone button
///
/// Any form may carry an `@ref` pin suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTarget {
    /// Host slot — alias or literal host, unresolved.
    pub host: Option<String>,
    pub org: Option<String>,
    pub repo: String,
    /// Optional `@ref` pin (tag, branch, or SHA).
    pub git_ref: Option<String>,
}

impl fmt::Display for ParsedTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.host, &self.org) {
            (Some(host), Some(org)) => write!(f, "//{}/{}:{}", host, org, self.repo)?,
            (Some(host), None) => write!(f, "//{}:{}", host, self.repo)?,
            (None, Some(org)) => write!(f, "//{}:{}", org, self.repo)?,
            (None, None) => write!(f, "{}", self.repo)?,
        }
        if let Some(git_ref) = &self.git_ref {
            write!(f, "@{}", git_ref)?;
        }
        Ok(())
    }
}
