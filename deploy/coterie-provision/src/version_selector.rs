use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub published_at: String,
    #[serde(default)]
    pub html_url: String,
}

pub fn parse_releases(json: &str) -> Result<Vec<Release>> {
    let releases: Vec<Release> =
        serde_json::from_str(json).context("failed to parse GitHub releases JSON")?;
    Ok(releases)
}

/// Return the most recent stable release (prerelease == false), or
/// None if every release in the list is a prerelease.
pub fn select_default_stable(releases: &[Release]) -> Option<&Release> {
    releases
        .iter()
        .filter(|r| !r.prerelease)
        .max_by(|a, b| a.published_at.cmp(&b.published_at))
}

/// Return up to N stable releases, sorted newest-first.
pub fn top_stable(releases: &[Release], limit: usize) -> Vec<&Release> {
    let mut stable: Vec<&Release> = releases.iter().filter(|r| !r.prerelease).collect();
    stable.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    stable.truncate(limit);
    stable
}

/// Return up to N releases (stable + prerelease) sorted newest-first.
pub fn top_all(releases: &[Release], limit: usize) -> Vec<&Release> {
    let mut all: Vec<&Release> = releases.iter().collect();
    all.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    all.truncate(limit);
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/github_releases.json");

    #[test]
    fn parses_release_fixture() {
        let releases = parse_releases(FIXTURE).expect("parses");
        assert_eq!(releases.len(), 5);
    }

    #[test]
    fn default_stable_is_newest_non_prerelease() {
        let releases = parse_releases(FIXTURE).expect("parses");
        let pick = select_default_stable(&releases).expect("at least one stable");
        assert_eq!(pick.tag_name, "v1.2.0");
        assert!(!pick.prerelease);
    }

    #[test]
    fn top_stable_excludes_prereleases() {
        let releases = parse_releases(FIXTURE).expect("parses");
        let stable = top_stable(&releases, 10);
        for r in &stable {
            assert!(!r.prerelease, "{} marked prerelease", r.tag_name);
        }
        assert_eq!(stable[0].tag_name, "v1.2.0");
    }

    #[test]
    fn top_all_includes_prereleases() {
        let releases = parse_releases(FIXTURE).expect("parses");
        let all = top_all(&releases, 10);
        assert_eq!(all[0].tag_name, "v1.3.0-rc1");
    }

    #[test]
    fn all_prerelease_yields_no_default() {
        let json = r#"[
            {"tag_name":"v1.0.0dev","prerelease":true,"published_at":"2026-01-01T00:00:00Z"}
        ]"#;
        let releases = parse_releases(json).expect("parses");
        assert!(select_default_stable(&releases).is_none());
    }
}
