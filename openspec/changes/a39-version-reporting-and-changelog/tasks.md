## 1. Version module (pure derivation + build-time values)

- [ ] 1.1 Add `src/version.rs` with `const RELEASE_TAG: Option<&str> = option_env!("COTERIE_VERSION");`
      and `const RELEASE_SHA: Option<&str> = option_env!("COTERIE_GIT_SHA");`.
- [ ] 1.2 Add a pure `pub fn display_version(tag: Option<&str>, sha: Option<&str>, crate_ver: &str) -> String`:
      `tag`+`sha` → `"<tag> (<sha>)"`; `tag` only → `"<tag>"`; no tag but `sha`
      → `format!("{crate_ver}-dev ({sha})")`; neither → `format!("{crate_ver}-dev")`.
- [ ] 1.3 Add `pub fn current() -> String` that calls `display_version(RELEASE_TAG, RELEASE_SHA, env!("CARGO_PKG_VERSION"))`,
      and `pub fn release_tag() -> Option<&'static str> { RELEASE_TAG }` for the
      portal link decision.
- [ ] 1.4 Add `pub fn reference() -> &'static str` returning the most specific
      ref: `RELEASE_TAG.or(RELEASE_SHA).unwrap_or("master")`.
- [ ] 1.5 Add `pub fn docs_url(path: &str) -> String` that builds
      `https://github.com/IndustriousKraken/coterie/blob/<reference()>/<path>`.
- [ ] 1.6 Add a `build.rs` at the crate root that, when `COTERIE_GIT_SHA` is NOT
      already set in the build env, runs `git rev-parse --short HEAD` and emits
      `cargo:rustc-env=COTERIE_GIT_SHA=<sha>`; emit `cargo:rerun-if-changed=.git/HEAD`.
      It MUST be a graceful no-op (leaving the var unset) when git or `.git` is
      absent, and MUST NOT set `COTERIE_VERSION`.
- [ ] 1.7 Declare `pub mod version;` in `src/lib.rs`.

## 2. Report the embedded version at the API surfaces

- [ ] 2.1 In `src/api/handlers/root.rs`, replace `env!("CARGO_PKG_VERSION")` in
      the `/api` info JSON (around line 67) with `crate::version::current()`.
- [ ] 2.2 Replace `env!("CARGO_PKG_VERSION")` in the health response (around
      line 129) with `crate::version::current()`.

## 3. Portal version line, release link, and version-pinned doc links

- [ ] 3.1 Add the running version to the portal base layout
      (`templates/layouts/base.html`) footer as an "about this version" line,
      fed by `BaseContext` (`src/web/templates`) so every portal page has it.
- [ ] 3.2 When `crate::version::release_tag()` is `Some(tag)`, link the version to
      `https://github.com/IndustriousKraken/coterie/releases/tag/<tag>`; when
      `None`, render the `-dev` string with no link.
- [ ] 3.3 On the Stripe/billing settings page (`templates/admin/billing_settings.html`,
      handler `src/web/portal/admin/billing.rs`), add a "Setup guide ↗" link to
      `crate::version::docs_url("docs/deploy/STRIPE-SETUP.md")`. Do NOT add doc links
      to the Discord or UniFi config pages — no dedicated guide exists for them.

## 4. CHANGELOG.md scaffold

- [ ] 4.1 Create `CHANGELOG.md` in Keep a Changelog format: title, a short note
      that released sections are immutable and the file is the source for release
      bodies, and an initial `## [Unreleased]` section with empty
      Added/Changed/Fixed subheads.
- [ ] 4.2 Include a brief comment documenting the release convention (rename
      `[Unreleased]` to `## [vX.Y.Z] — YYYY-MM-DD`, commit, then tag) so the
      changelog generator and maintainers share one target.

## 5. Release workflow: embed version + source the body from the changelog

- [ ] 5.1 In `.github/workflows/release.yml`, set `COTERIE_VERSION=${{ steps.version.outputs.tag }}`
      and `COTERIE_GIT_SHA=${{ steps.version.outputs.sha }}` as env on the main
      `cargo build --release` step (the `coterie` build, ~line 103) so the shipped
      binary embeds them.
- [ ] 5.2 Add a step that extracts the `CHANGELOG.md` section for
      `${{ steps.version.outputs.tag }}` into a file (e.g. `awk` capturing lines
      from the matching `## [<tag>]` header up to the next `## [` header).
- [ ] 5.3 In the `softprops/action-gh-release@v2` step, set `body_path` to the
      extracted file. Keep `generate_release_notes: true` so notes are appended /
      used when the extracted section is empty, ensuring a non-empty body.

## 6. README development banner

- [ ] 6.1 Add a one-line notice near the top of `README.md`: the docs track the
      latest development version; on an older release, see that release's notes
      (Releases page) or browse the repo at your tag for matching docs.

## 7. Tests

- [ ] 7.1 Unit tests for `display_version`: tag-plus-SHA, tag-without-SHA,
      no-tag-with-SHA (`0.1.0-dev (abc1234)`), and neither (`0.1.0-dev`),
      asserting the exact rendered string for each.
- [ ] 7.2 Unit test for the ref-selection used by `docs_url`: tag wins over SHA,
      SHA used when no tag, `master` when neither — yielding
      `…/blob/<tag|sha|master>/<path>`. Drive selection via a pure helper taking
      the optional tag/SHA so the test doesn't depend on build-time env.
- [ ] 7.3 An integration test asserting `GET /health` returns a `version` field
      equal to `crate::version::current()` (the dev fallback under test build).

## 8. Validation

- [ ] 8.1 `cargo build` — clean.
- [ ] 8.2 `cargo test` — all pass, including the new version unit tests and the
      `/health` assertion.
- [ ] 8.3 `cargo clippy --all-targets -- --deny warnings` — clean.
- [ ] 8.4 `cargo fmt --check` — clean.
- [ ] 8.5 `bash -n` any shell added to the release workflow extraction step (if a
      standalone script is used).
- [ ] 8.6 `openspec validate a39-version-reporting-and-changelog --strict` — clean.
