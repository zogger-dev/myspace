use crate::models::config::GlobalConfig;
use crate::models::target::Target;
use anyhow::{Result, anyhow};

/// Parses a string into a Target enum based on slashes.
pub fn parse_target(s: &str) -> Result<Target> {
    if let Some(trimmed) = s.strip_prefix("//") {
        let parts: Vec<&str> = trimmed.split('/').collect();
        if parts.len() != 3 {
            return Err(anyhow!(
                "Invalid aliased target format. Expected //alias/org/repo"
            ));
        }
        Ok(Target::Aliased(
            parts[0].to_string(),
            parts[1].to_string(),
            parts[2].to_string(),
        ))
    } else {
        let parts: Vec<&str> = s.split('/').collect();
        match parts.len() {
            1 => Ok(Target::Bare(parts[0].to_string())),
            2 => Ok(Target::OrgRepo(parts[0].to_string(), parts[1].to_string())),
            _ => Err(anyhow!("Invalid target format. Expected repo or org/repo")),
        }
    }
}

/// Resolves a Target into a full Git URL using the GlobalConfig.
pub fn resolve_git_url(target: &Target, config: &GlobalConfig) -> Result<String> {
    match target {
        Target::Bare(repo) => Ok(format!(
            "{}:{}/{}.git",
            config.default_scm, config.default_org, repo
        )),
        Target::OrgRepo(org, repo) => Ok(format!("{}:{}/{}.git", config.default_scm, org, repo)),
        Target::Aliased(alias, org, repo) => {
            let scm = config
                .aliases
                .get(alias)
                .ok_or_else(|| anyhow!("Alias '{}' not found in global configuration", alias))?;
            Ok(format!("{}:{}/{}.git", scm, org, repo))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn get_test_config() -> GlobalConfig {
        let mut aliases = HashMap::new();
        aliases.insert("bb".to_string(), "git@bitbucket.org".to_string());

        GlobalConfig {
            default_org: "zogger-dev".to_string(),
            default_scm: "git@github.com".to_string(),
            aliases,
        }
    }

    #[test]
    fn test_parse_bare() {
        assert_eq!(
            parse_target("repo").unwrap(),
            Target::Bare("repo".to_string())
        );
    }

    #[test]
    fn test_parse_org_repo() {
        assert_eq!(
            parse_target("apache/impala").unwrap(),
            Target::OrgRepo("apache".to_string(), "impala".to_string())
        );
    }

    #[test]
    fn test_parse_aliased() {
        assert_eq!(
            parse_target("//bb/myorg/myrepo").unwrap(),
            Target::Aliased("bb".to_string(), "myorg".to_string(), "myrepo".to_string())
        );
    }

    #[test]
    fn test_resolve_bare() {
        let config = get_test_config();
        let target = Target::Bare("repo".to_string());
        let url = resolve_git_url(&target, &config).unwrap();
        assert_eq!(url, "git@github.com:zogger-dev/repo.git");
    }

    #[test]
    fn test_resolve_org_repo() {
        let config = get_test_config();
        let target = Target::OrgRepo("apache".to_string(), "impala".to_string());
        let url = resolve_git_url(&target, &config).unwrap();
        assert_eq!(url, "git@github.com:apache/impala.git");
    }

    #[test]
    fn test_resolve_aliased() {
        let config = get_test_config();
        let target = Target::Aliased("bb".to_string(), "myorg".to_string(), "myrepo".to_string());
        let url = resolve_git_url(&target, &config).unwrap();
        assert_eq!(url, "git@bitbucket.org:myorg/myrepo.git");
    }
}
