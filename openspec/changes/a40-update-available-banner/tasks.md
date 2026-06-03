## 1. Version comparison

- [ ] 1.1 Add a pure `pub fn is_newer(candidate: &str, running: &str) -> bool`
      (new module, e.g. `src/service/update_check.rs`) that strips a leading `v`,
      parses `X.Y.Z`, and returns whether `candidate` is strictly newer. Use the
      `semver` crate or a minimal parser. Unparseable input → `false` (ignored,
      never "newer").

## 2. Release fetch + latest-stable selection

- [ ] 2.1 Add an async fetch that GETs the GitHub releases list for
      `IndustriousKraken/coterie` via the existing `reqwest` dependency and
      deserializes `tag_name`, `prerelease`, and `html_url`.
- [ ] 2.2 Add selection that returns the most recent release with
      `prerelease == false` (compare by `is_newer` among stable tags). Return
      `None` on fetch/parse error (caller treats as "no update info").

## 3. AppState cache

- [ ] 3.1 Add a cache field to `AppState` (`src/api/state.rs`), e.g.
      `Arc<RwLock<Option<LatestRelease>>>` where `LatestRelease { tag, notes_url }`.
- [ ] 3.2 Add the matching `FromRef<AppState>` impl (per the convention comment
      in `state.rs`) and a constructor wire-up in `AppState::new`.

## 4. Background check task

- [ ] 4.1 In `src/main.rs`, spawn a task mirroring the reconcile/horizon loops:
      an initial short delay, then every 24h.
- [ ] 4.2 Each cycle: read `updates.check_enabled`; if disabled, clear the cache
      and skip the fetch. If enabled, fetch + select latest stable and write it to
      the cache. On error, leave the cache unchanged and log at warn/debug.

## 5. Setting

- [ ] 5.1 Add the `updates.check_enabled` setting (boolean, default `true`) to
      the settings defaults/seed.
- [ ] 5.2 Surface it as a toggle in the admin settings UI
      (`src/web/portal/admin/settings.rs` + its template) with a one-line note
      that enabling it contacts the public GitHub releases API.

## 6. Banner + dismiss

- [ ] 6.1 Add an admin-gated "update available" banner to
      `templates/layouts/base.html`, fed by a render-context value that is
      `Some` only when: the session is admin, the running build has a release tag
      (`crate::version::release_tag().is_some()`), the cache holds a stable tag,
      and `is_newer(cached, running)` is true.
- [ ] 6.2 The banner links to the cached release notes URL and to the README
      "Update" steps (use `crate::version::docs_url("README.md")` with an
      `#update` anchor, or the Releases page).
- [ ] 6.3 Add a dismiss control that sets an `update_dismissed=<tag>` cookie
      (client-side is fine — no new route). Suppress the banner when the cookie
      value equals the cached latest tag.

## 7. Tests

- [ ] 7.1 Unit tests for `is_newer`: `v1.10.0 > v1.9.0`, equal → false, older →
      false, leading-`v` tolerance, and unparseable → false.
- [ ] 7.2 Unit test for latest-stable selection: a prerelease that is numerically
      highest is NOT selected over the highest stable.
- [ ] 7.3 Render/integration tests against an injected cache value: admin + newer
      cached tag → banner shown; member → hidden; build with no release tag →
      hidden; `update_dismissed` cookie equal to the cached tag → hidden, and a
      newer cached tag → shown again.

## 8. Validation

- [ ] 8.1 `cargo build` — clean.
- [ ] 8.2 `cargo test` — all pass, including the new unit and render tests.
- [ ] 8.3 `cargo clippy --all-targets -- --deny warnings` — clean.
- [ ] 8.4 `cargo fmt --check` — clean.
- [ ] 8.5 `openspec validate a40-update-available-banner --strict` — clean.
