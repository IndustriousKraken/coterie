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
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    #[serde(default)]
    pub browser_download_url: String,
}

/// Parse a GitHub `/releases` API blob (an array of releases).
pub fn parse_releases(json: &str) -> Result<Vec<Release>> {
    serde_json::from_str(json).context("failed to parse releases JSON")
}

/// Pick the default-stable release: the highest `published_at` among
/// releases where `prerelease == false`.
pub fn select_default_stable(releases: &[Release]) -> Option<&Release> {
    releases
        .iter()
        .filter(|r| !r.prerelease)
        .max_by(|a, b| a.published_at.cmp(&b.published_at))
}

/// Return the most recent `limit` stable releases, newest first.
pub fn top_stable(releases: &[Release], limit: usize) -> Vec<&Release> {
    let mut stable: Vec<&Release> = releases.iter().filter(|r| !r.prerelease).collect();
    stable.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    stable.into_iter().take(limit).collect()
}

/// Return all recent releases (including prereleases), newest first.
pub fn top_all(releases: &[Release], limit: usize) -> Vec<&Release> {
    let mut all: Vec<&Release> = releases.iter().collect();
    all.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    all.into_iter().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> &'static str {
        include_str!("../tests/fixtures/github_releases.json")
    }

    #[test]
    fn parses_fixture() {
        let releases = parse_releases(fixture()).unwrap();
        assert!(releases.len() >= 4);
    }

    #[test]
    fn default_stable_skips_prereleases() {
        let releases = parse_releases(fixture()).unwrap();
        let pick = select_default_stable(&releases).expect("a stable exists");
        assert!(!pick.prerelease);
        assert_eq!(pick.tag_name, "v1.1.0");
    }

    #[test]
    fn top_stable_is_sorted_newest_first() {
        let releases = parse_releases(fixture()).unwrap();
        let top = top_stable(&releases, 5);
        assert_eq!(top[0].tag_name, "v1.1.0");
        assert_eq!(top[1].tag_name, "v1.0.0");
    }

    #[test]
    fn top_all_includes_prereleases() {
        let releases = parse_releases(fixture()).unwrap();
        let top = top_all(&releases, 10);
        // The newest item is a prerelease.
        assert!(top[0].prerelease);
        assert_eq!(top[0].tag_name, "v1.2.0-rc1");
    }
}
