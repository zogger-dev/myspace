use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Bare repo: e.g. "repo"
    Bare(String),
    /// Org and repo: e.g. "org/repo"
    OrgRepo(String, String),
    /// Alias, org and repo: e.g. "//alias/org/repo"
    Aliased(String, String, String),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Bare(repo) => write!(f, "{}", repo),
            Target::OrgRepo(org, repo) => write!(f, "{}/{}", org, repo),
            Target::Aliased(alias, org, repo) => write!(f, "//{}/{}/{}", alias, org, repo),
        }
    }
}
