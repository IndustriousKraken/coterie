//! Background "is a newer stable release available?" check, plus the
//! pure version comparator and banner-decision logic the portal reads.
//!
//! Resilience contract (`docs/ARCHITECTURE.md`): no outbound network
//! call in the request path. The daily background task talks to GitHub
//! and writes the [`cache`]; the portal render path only ever reads the
//! cached value via [`cached_latest`]. A GitHub outage leaves the cache
//! unchanged and never surfaces an error to a page (a40 / D8).
//!
//! All decision logic is pure (`is_newer`, `select_latest_stable`,
//! `banner_for`) so it is unit-testable without the network or
//! build-time state.

use std::sync::{Arc, OnceLock, RwLock};

use serde::Deserialize;

/// GitHub `/releases` endpoint for the canonical repo. `per_page=20`
/// is plenty to find the latest stable even when recent prereleases
/// sit on top of it.
const RELEASES_URL: &str =
    "https://api.github.com/repos/IndustriousKraken/coterie/releases?per_page=20";

/// GitHub's API requires a User-Agent header; requests without one are
/// rejected. Identifies the project per their guidance.
const USER_AGENT: &str = "Coterie (https://github.com/IndustriousKraken/coterie)";

/// The cached latest *stable* release, in the shape the banner needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestRelease {
    /// The release tag, e.g. `v1.4.0`.
    pub tag: String,
    /// The release notes URL (GitHub `html_url`).
    pub notes_url: String,
}

/// What the portal needs to render the update-available banner: the
/// newer stable tag and a link to its notes. The "how to update" link
/// is version-pinned and built by the render layer, not here, so this
/// stays free of build-time state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateBanner {
    pub tag: String,
    pub notes_url: String,
}

/// One release as returned by the GitHub `/releases` API. Only the
/// fields the check uses; `#[serde(default)]` keeps parsing resilient
/// to absent keys rather than failing the whole list.
#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    html_url: String,
}

/// Process-wide cache handle. The daily task writes it; the static
/// render helper (`BaseContext::for_member`) reads it. `AppState` also
/// holds this same `Arc` (see `api::state::LatestReleaseCache`) for a
/// canonical home and a `FromRef` impl, but the backing store is this
/// singleton so the render path can read what the task writes without
/// threading the cache through every portal handler.
static CACHE: OnceLock<Arc<RwLock<Option<LatestRelease>>>> = OnceLock::new();

/// The shared cache handle, lazily initialized to "nothing cached yet".
pub fn cache() -> Arc<RwLock<Option<LatestRelease>>> {
    CACHE.get_or_init(|| Arc::new(RwLock::new(None))).clone()
}

/// A clone of the currently-cached latest stable release, if any.
/// Recovers from a poisoned lock rather than panicking — a stale or
/// absent hint is harmless, a panicked render is not.
pub fn cached_latest() -> Option<LatestRelease> {
    let handle = cache();
    let guard = handle.read().unwrap_or_else(|p| p.into_inner());
    guard.clone()
}

/// Overwrite the cache (or clear it with `None`). Recovers from a
/// poisoned lock.
pub fn store(value: Option<LatestRelease>) {
    let handle = cache();
    let mut guard = handle.write().unwrap_or_else(|p| p.into_inner());
    *guard = value;
}

/// Build the HTTP client the background check uses: the GitHub-required
/// User-Agent and a short timeout so a hung connection can't wedge the
/// daily loop.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Parse a `vX.Y.Z` tag into a `(major, minor, patch)` triple. A
/// leading `v`/`V` is stripped. Returns `None` for anything that is not
/// exactly three dotted integers — including prerelease-suffixed tags
/// like `v1.2.0-rc1` (the `0-rc1` patch fails to parse) and tags
/// missing a component. Such tags are *ignored* rather than treated as
/// an error or as "newer".
fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let trimmed = tag
        .strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(tag);
    let mut parts = trimmed.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    // A fourth component means this isn't a clean `X.Y.Z` — ignore it.
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Whether `candidate` is a strictly newer release than `running`. Both
/// are `vX.Y.Z` tags (leading `v` optional). Comparison is numeric per
/// component (so `v1.10.0` > `v1.9.0`, not a string compare). If either
/// side fails to parse, returns `false` — an unparseable tag is never
/// treated as newer, so it can't trigger the banner.
pub fn is_newer(candidate: &str, running: &str) -> bool {
    match (parse_version(candidate), parse_version(running)) {
        (Some(c), Some(r)) => c > r,
        _ => false,
    }
}

/// Select the latest *stable* release from a parsed list: among
/// releases with `prerelease == false`, the highest version per
/// [`is_newer`]. Stable tags that don't parse are skipped (they can't
/// be compared and must not win by accident). Returns `None` when the
/// list holds no parseable stable release. A numerically-higher
/// prerelease never wins over the highest stable, because prereleases
/// are filtered out entirely.
fn select_latest_stable(releases: &[GithubRelease]) -> Option<LatestRelease> {
    let mut best: Option<&GithubRelease> = None;
    for release in releases.iter().filter(|r| !r.prerelease) {
        if parse_version(&release.tag_name).is_none() {
            continue;
        }
        match best {
            None => best = Some(release),
            Some(current) if is_newer(&release.tag_name, &current.tag_name) => best = Some(release),
            _ => {}
        }
    }
    best.map(|r| LatestRelease {
        tag: r.tag_name.clone(),
        notes_url: r.html_url.clone(),
    })
}

/// Fetch the releases list and return the latest stable, or `None` on
/// any network/parse/non-2xx error. The caller treats `None` as "no
/// update info" and leaves the cache unchanged (graceful degradation).
pub async fn fetch_latest_stable(client: &reqwest::Client) -> Option<LatestRelease> {
    match fetch_releases(client).await {
        Ok(releases) => select_latest_stable(&releases),
        Err(e) => {
            // Debug, not error: a GitHub hiccup is expected and harmless.
            tracing::debug!("update check: could not fetch releases: {e}");
            None
        }
    }
}

async fn fetch_releases(client: &reqwest::Client) -> Result<Vec<GithubRelease>, reqwest::Error> {
    client
        .get(RELEASES_URL)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .json::<Vec<GithubRelease>>()
        .await
}

/// Decide whether to show the update banner. Pure, so it is unit-testable
/// without rendering or build-time state — this is the spec's canonical,
/// unit-tested decision (a40).
///
/// Returns `Some` only when ALL hold:
/// * the session is an admin (members never see it),
/// * the running build has an embedded release tag (a dev/untagged
///   build has nothing meaningful to compare and never nags),
/// * a stable release is cached, and
/// * the cached tag is strictly newer than the running tag, and
/// * the banner has not been dismissed for that exact tag.
///
/// `dismissed` is the value of the `update_dismissed` cookie (the tag an
/// admin last dismissed), or `None`. In the live render path dismissal
/// is enforced client-side from the cookie (see `templates/layouts/
/// base.html`), so `for_member` passes `None`; this parameter keeps the
/// full rule — including the dismiss-until-newer behavior — in one
/// place so it can be unit-tested directly.
pub fn banner_for(
    is_admin: bool,
    running_tag: Option<&str>,
    cached: Option<&LatestRelease>,
    dismissed: Option<&str>,
) -> Option<UpdateBanner> {
    if !is_admin {
        return None;
    }
    let running = running_tag?;
    let cached = cached?;
    if !is_newer(&cached.tag, running) {
        return None;
    }
    // Dismissed for this exact tag → stay hidden. A newer cached tag no
    // longer matches the dismissed one, so the banner returns.
    if dismissed == Some(cached.tag.as_str()) {
        return None;
    }
    Some(UpdateBanner {
        tag: cached.tag.clone(),
        notes_url: cached.notes_url.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool) -> GithubRelease {
        GithubRelease {
            tag_name: tag.to_string(),
            prerelease,
            html_url: format!("https://example.test/releases/{tag}"),
        }
    }

    // ---- is_newer ----

    #[test]
    fn newer_across_a_digit_boundary() {
        // The whole point of a numeric compare: 10 > 9, even though
        // "1.10.0" < "1.9.0" as strings.
        assert!(is_newer("v1.10.0", "v1.9.0"));
    }

    #[test]
    fn equal_is_not_newer() {
        assert!(!is_newer("v1.2.3", "v1.2.3"));
    }

    #[test]
    fn older_is_not_newer() {
        assert!(!is_newer("v1.2.2", "v1.2.3"));
        assert!(!is_newer("v0.9.0", "v1.0.0"));
    }

    #[test]
    fn leading_v_is_optional_on_either_side() {
        assert!(is_newer("1.3.0", "v1.2.9"));
        assert!(is_newer("v1.3.0", "1.2.9"));
        assert!(!is_newer("V1.0.0", "v1.0.0"));
    }

    #[test]
    fn unparseable_is_never_newer() {
        // Garbage on either side → false, never "newer".
        assert!(!is_newer("not-a-version", "v1.0.0"));
        assert!(!is_newer("v2.0.0", "garbage"));
        assert!(!is_newer("v1.2", "v1.1.0")); // missing patch
        assert!(!is_newer("v1.2.0-rc1", "v1.1.0")); // prerelease suffix
    }

    // ---- latest-stable selection ----

    #[test]
    fn latest_stable_skips_a_higher_prerelease() {
        // v1.5.0-rc1 is numerically the highest but is a prerelease, so
        // selection must return the highest *stable*, v1.4.0.
        let releases = vec![
            release("v1.5.0-rc1", true),
            release("v1.4.0", false),
            release("v1.3.0", false),
        ];
        let picked = select_latest_stable(&releases).expect("a stable exists");
        assert_eq!(picked.tag, "v1.4.0");
        assert_eq!(picked.notes_url, "https://example.test/releases/v1.4.0");
    }

    #[test]
    fn latest_stable_orders_numerically_not_by_position() {
        // Out of order and crossing a digit boundary.
        let releases = vec![
            release("v1.9.0", false),
            release("v1.10.0", false),
            release("v1.2.0", false),
        ];
        let picked = select_latest_stable(&releases).expect("a stable exists");
        assert_eq!(picked.tag, "v1.10.0");
    }

    #[test]
    fn latest_stable_skips_unparseable_stable_tags() {
        let releases = vec![release("nightly", false), release("v1.1.0", false)];
        let picked = select_latest_stable(&releases).expect("a stable exists");
        assert_eq!(picked.tag, "v1.1.0");
    }

    #[test]
    fn latest_stable_is_none_with_only_prereleases() {
        let releases = vec![release("v2.0.0-rc1", true), release("v2.0.0-beta", true)];
        assert!(select_latest_stable(&releases).is_none());
    }

    #[test]
    fn releases_json_deserializes_used_fields() {
        let json = r#"[
            {"tag_name":"v1.4.0","prerelease":false,"html_url":"https://gh/r/v1.4.0"},
            {"tag_name":"v1.5.0-rc1","prerelease":true,"html_url":"https://gh/r/v1.5.0-rc1"}
        ]"#;
        let parsed: Vec<GithubRelease> = serde_json::from_str(json).unwrap();
        let picked = select_latest_stable(&parsed).expect("a stable exists");
        assert_eq!(picked.tag, "v1.4.0");
        assert_eq!(picked.notes_url, "https://gh/r/v1.4.0");
    }

    // ---- banner_for: the render decision (covers task 7.3 at the
    // function level: admin+newer shown; member hidden; dev build
    // hidden; dismissed==cached hidden; newer-than-dismissed shown) ----

    fn cached(tag: &str) -> LatestRelease {
        LatestRelease {
            tag: tag.to_string(),
            notes_url: format!("https://gh/r/{tag}"),
        }
    }

    #[test]
    fn banner_shows_for_admin_on_a_newer_release() {
        let c = cached("v1.4.0");
        let banner = banner_for(true, Some("v1.3.0"), Some(&c), None)
            .expect("admin behind a newer stable should see the banner");
        assert_eq!(banner.tag, "v1.4.0");
        assert_eq!(banner.notes_url, "https://gh/r/v1.4.0");
    }

    #[test]
    fn banner_hidden_for_non_admin() {
        let c = cached("v1.4.0");
        assert!(banner_for(false, Some("v1.3.0"), Some(&c), None).is_none());
    }

    #[test]
    fn banner_hidden_for_dev_build_without_a_release_tag() {
        let c = cached("v1.4.0");
        assert!(banner_for(true, None, Some(&c), None).is_none());
    }

    #[test]
    fn banner_hidden_when_up_to_date_or_ahead() {
        let c = cached("v1.4.0");
        assert!(banner_for(true, Some("v1.4.0"), Some(&c), None).is_none());
        assert!(banner_for(true, Some("v1.5.0"), Some(&c), None).is_none());
    }

    #[test]
    fn banner_hidden_with_nothing_cached() {
        assert!(banner_for(true, Some("v1.3.0"), None, None).is_none());
    }

    #[test]
    fn banner_hidden_when_dismissed_for_that_tag() {
        let c = cached("v1.4.0");
        assert!(banner_for(true, Some("v1.3.0"), Some(&c), Some("v1.4.0")).is_none());
    }

    #[test]
    fn banner_reappears_when_a_newer_release_supersedes_the_dismissed_one() {
        // Dismissed v1.4.0, but the cache has advanced to v1.5.0.
        let c = cached("v1.5.0");
        let banner = banner_for(true, Some("v1.3.0"), Some(&c), Some("v1.4.0"))
            .expect("a release newer than the dismissed one re-shows the banner");
        assert_eq!(banner.tag, "v1.5.0");
    }

    // ---- the process-wide cache round-trips ----

    #[test]
    fn cache_store_and_read_round_trip() {
        store(Some(cached("v9.9.9")));
        assert_eq!(cached_latest().map(|r| r.tag), Some("v9.9.9".to_string()));
        store(None);
        assert!(cached_latest().is_none());
    }
}
