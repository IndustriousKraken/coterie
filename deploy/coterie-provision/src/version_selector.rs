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
/// releases where `prerelease == false`. This is the "latest stable"
/// selection the update flow uses when no `--tag` is supplied.
pub fn select_default_stable(releases: &[Release]) -> Option<&Release> {
    releases
        .iter()
        .filter(|r| !r.prerelease)
        .max_by(|a, b| a.published_at.cmp(&b.published_at))
}

/// Find a release by exact tag name. Returns `None` when no release in
/// the list carries that tag. Used by the update flow to honor an
/// explicit `--tag <vX.Y.Z>` (rollback or pinned-version) request.
pub fn find_by_tag<'a>(releases: &'a [Release], tag: &str) -> Option<&'a Release> {
    releases.iter().find(|r| r.tag_name == tag)
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

    #[test]
    fn latest_stable_skips_a_newer_prerelease() {
        // The newest release in the fixture (v1.2.0-rc1) is a prerelease;
        // latest-stable resolution must skip it and pick v1.1.0.
        let releases = parse_releases(fixture()).unwrap();
        let newest = top_all(&releases, 1)[0];
        assert!(newest.prerelease, "fixture's newest must be a prerelease");
        let stable = select_default_stable(&releases).expect("a stable exists");
        assert!(!stable.prerelease);
        assert_eq!(stable.tag_name, "v1.1.0");
    }

    #[test]
    fn find_by_tag_hits_an_existing_release() {
        let releases = parse_releases(fixture()).unwrap();
        let found = find_by_tag(&releases, "v1.0.0").expect("v1.0.0 is present");
        assert_eq!(found.tag_name, "v1.0.0");
        assert!(!found.prerelease);
    }

    #[test]
    fn find_by_tag_can_pin_a_prerelease() {
        // `--tag` is an exact match, so it may pin a prerelease the
        // default-stable path would never auto-select.
        let releases = parse_releases(fixture()).unwrap();
        let found = find_by_tag(&releases, "v1.2.0-rc1").expect("rc1 is present");
        assert!(found.prerelease);
    }

    #[test]
    fn find_by_tag_misses_unknown_tag() {
        let releases = parse_releases(fixture()).unwrap();
        assert!(find_by_tag(&releases, "v9.9.9").is_none());
    }
}
