//! Build-time version reporting.
//!
//! The crate version (`CARGO_PKG_VERSION`) is the constant `0.1.0` and
//! says nothing about which release an operator is actually running.
//! This module gives the running binary a correct anchor to its own
//! version by reading two build-time environment variables:
//!
//! * `COTERIE_VERSION` — the release tag (e.g. `v1.2.3`). Set only by
//!   the release workflow; a tag asserts "a release exists," which is a
//!   claim only that workflow may make.
//! * `COTERIE_GIT_SHA` — the short commit SHA. Set by the release
//!   workflow, or stamped by `build.rs` from `git rev-parse` for
//!   untagged local/staging builds.
//!
//! Both are read with [`option_env!`], so a build with neither set
//! (a source archive with no `.git`, say) falls back gracefully to a
//! `-dev` marker. All derivation logic is pure functions taking their
//! inputs as arguments, so it is fully unit-testable without
//! manipulating build-time environment variables.

/// Canonical repository URL. Every link the binary emits (release page,
/// version-pinned docs) is built from this single constant.
const REPO_URL: &str = "https://github.com/IndustriousKraken/coterie";

/// The release tag embedded at build time, if any. `Some` only for
/// builds produced by the release workflow.
const RELEASE_TAG: Option<&str> = option_env!("COTERIE_VERSION");

/// The short commit SHA embedded at build time, if any. Set by the
/// release workflow or stamped by `build.rs` for untagged builds.
const RELEASE_SHA: Option<&str> = option_env!("COTERIE_GIT_SHA");

/// Render a human-readable version string from its parts.
///
/// Pure — all inputs are passed in, so every branch is unit-testable
/// without touching build-time environment variables.
///
/// * tag + sha → `"<tag> (<sha>)"`
/// * tag only → `"<tag>"`
/// * no tag, sha → `"<crate_ver>-dev (<sha>)"`
/// * neither → `"<crate_ver>-dev"`
pub fn display_version(tag: Option<&str>, sha: Option<&str>, crate_ver: &str) -> String {
    match (tag, sha) {
        (Some(tag), Some(sha)) => format!("{tag} ({sha})"),
        (Some(tag), None) => tag.to_string(),
        (None, Some(sha)) => format!("{crate_ver}-dev ({sha})"),
        (None, None) => format!("{crate_ver}-dev"),
    }
}

/// The version string for the running binary, e.g. `"v1.2.3 (abc1234)"`
/// for a release build or `"0.1.0-dev (abc1234)"` for a dev build.
pub fn current() -> String {
    display_version(RELEASE_TAG, RELEASE_SHA, env!("CARGO_PKG_VERSION"))
}

/// The embedded release tag, if this is a real release build. Used by
/// the portal to decide whether to link the version to a release page —
/// the decision is "is a tag embedded," not "does the version string
/// look like a tag."
pub fn release_tag() -> Option<&'static str> {
    RELEASE_TAG
}

/// The GitHub release page URL for a tag.
pub fn release_url(tag: &str) -> String {
    format!("{REPO_URL}/releases/tag/{tag}")
}

/// Select the most specific Git ref for a build from its optional tag
/// and SHA: tag wins over SHA, SHA over the default branch. Pure so the
/// selection is testable without build-time env.
fn select_reference<'a>(tag: Option<&'a str>, sha: Option<&'a str>) -> &'a str {
    tag.or(sha).unwrap_or("master")
}

/// Build a `blob/<reference>/<path>` documentation URL for a given ref.
/// Pure helper so the full URL shape is testable without build-time env.
fn build_docs_url(reference: &str, path: &str) -> String {
    format!("{REPO_URL}/blob/{reference}/{path}")
}

/// The most specific Git ref we can resolve for the running build:
/// release tag, else commit SHA, else the default branch (`master`).
pub fn reference() -> &'static str {
    select_reference(RELEASE_TAG, RELEASE_SHA)
}

/// A version-pinned link to a repository document. Pins to the running
/// build's ref so the linked doc matches the running build rather than
/// always showing whatever is on `master`.
pub fn docs_url(path: &str) -> String {
    build_docs_url(reference(), path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- display_version: all four branches, exact strings ----

    #[test]
    fn display_version_tag_and_sha() {
        assert_eq!(
            display_version(Some("v1.2.3"), Some("abc1234"), "0.1.0"),
            "v1.2.3 (abc1234)"
        );
    }

    #[test]
    fn display_version_tag_without_sha() {
        assert_eq!(display_version(Some("v1.2.3"), None, "0.1.0"), "v1.2.3");
    }

    #[test]
    fn display_version_no_tag_with_sha() {
        assert_eq!(
            display_version(None, Some("abc1234"), "0.1.0"),
            "0.1.0-dev (abc1234)"
        );
    }

    #[test]
    fn display_version_neither() {
        assert_eq!(display_version(None, None, "0.1.0"), "0.1.0-dev");
    }

    // ---- ref selection / docs_url: tag → sha → master priority ----

    #[test]
    fn reference_prefers_tag_over_sha() {
        assert_eq!(select_reference(Some("v1.2.3"), Some("abc1234")), "v1.2.3");
        assert_eq!(
            build_docs_url(
                select_reference(Some("v1.2.3"), Some("abc1234")),
                "docs/deploy/STRIPE-SETUP.md"
            ),
            "https://github.com/IndustriousKraken/coterie/blob/v1.2.3/docs/deploy/STRIPE-SETUP.md"
        );
    }

    #[test]
    fn reference_uses_sha_when_no_tag() {
        assert_eq!(select_reference(None, Some("abc1234")), "abc1234");
        assert_eq!(
            build_docs_url(select_reference(None, Some("abc1234")), "docs/x.md"),
            "https://github.com/IndustriousKraken/coterie/blob/abc1234/docs/x.md"
        );
    }

    #[test]
    fn reference_falls_back_to_master() {
        assert_eq!(select_reference(None, None), "master");
        assert_eq!(
            build_docs_url(select_reference(None, None), "docs/x.md"),
            "https://github.com/IndustriousKraken/coterie/blob/master/docs/x.md"
        );
    }

    #[test]
    fn release_url_points_at_the_tag() {
        assert_eq!(
            release_url("v1.2.3"),
            "https://github.com/IndustriousKraken/coterie/releases/tag/v1.2.3"
        );
    }
}
