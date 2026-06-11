//! Build script: stamp the short commit SHA for untagged builds.
//!
//! The release workflow sets `COTERIE_VERSION` (the tag) and
//! `COTERIE_GIT_SHA` (the short SHA) explicitly, and its values must
//! always win. For everything else — local dev, the staging
//! `deploy.yml` — this script probes git so an untagged build still
//! embeds a real commit and reads e.g. `0.1.0-dev (abc1234)` rather
//! than a bare `-dev`.
//!
//! It is a graceful no-op when git or `.git` is absent (e.g. a source
//! archive), leaving the variable unset so `option_env!` resolves to
//! `None`. It never sets `COTERIE_VERSION`: a tag asserts "a release
//! exists," which only the release workflow may claim.

use std::process::Command;

fn main() {
    // Re-run when HEAD moves (new commit / checkout) so the stamped
    // SHA stays current without a `cargo clean`.
    println!("cargo:rerun-if-changed=.git/HEAD");

    // The release workflow already set the SHA in the build env —
    // never override it.
    if std::env::var_os("COTERIE_GIT_SHA").is_some() {
        return;
    }

    if let Some(sha) = git_short_sha() {
        println!("cargo:rustc-env=COTERIE_GIT_SHA={sha}");
    }
    // No git / no `.git`: leave COTERIE_GIT_SHA unset. Never set
    // COTERIE_VERSION here.
}

/// `git rev-parse --short HEAD`, or `None` if git is unavailable, the
/// command fails (no `.git`), or the output is empty.
fn git_short_sha() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}
