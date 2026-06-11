## Why

An operator running an older release has no in-product signal that a newer
stable version exists — they'd have to go check GitHub. With a39 the instance
now knows its own version and a38 makes updating easy; an admin-facing "update
available" banner closes the loop and helps keep security-sensitive instances
(member data, payments) current. Showing newer versions to a behind operator is
exactly the right place for the "you're behind" nudge — provided it's
version-aware, stable-only, and admin-only.

## What Changes

- **Background update check.** A daily task (same `tokio::spawn` + initial-delay
  pattern as the Discord reconcile / horizon jobs) fetches the GitHub releases
  list, selects the latest **non-prerelease** release, and caches it on
  `AppState`. It never runs in the request path and degrades silently when
  GitHub is unreachable.
- **Admin-only "update available" banner.** Rendered in the portal when the
  cached latest stable release is newer than the running release. Members never
  see it; development/untagged builds (no embedded release tag) never trigger it.
  The banner links to the release notes and the README "Update" steps.
- **Dismissible until a newer version ships.** A dismiss control records the
  dismissed version in a cookie; the banner stays hidden until a release newer
  than the dismissed one appears.
- **Opt-out setting** `updates.check_enabled` (default on). When off, no fetch
  happens and no banner is shown.
- **Pure, unit-tested version comparison** that excludes prereleases.

## Capabilities

### New Capabilities
- `update-notification`: a background check for newer stable releases and an
  admin-only, dismissible in-portal banner that surfaces them — stable-only,
  release-builds-only, opt-out, and resilient to a GitHub outage.

### Modified Capabilities
<!-- None. Reuses a39's `version` module to know the running release; adds a new
     AppState field and a new background task without changing existing
     requirements. -->

## Impact

- **Depends on a39** (`crate::version::release_tag()` / `current()` to know the
  running release). Ships after a39.
- **Code**: `src/api/state.rs` (cached latest-stable field + `FromRef`); a new
  check module (release fetch + latest-stable selection + comparator, using the
  existing `reqwest` dependency); `src/main.rs` (spawn the daily task);
  `templates/layouts/base.html` (admin-gated banner + dismiss control).
- **Settings**: new `updates.check_enabled` key (default on), surfaced in the
  admin settings UI.
- **Deps**: a small semver comparison — either the `semver` crate or a minimal
  `vX.Y.Z` parser.
- **No DB schema change** (the cache is in-memory; the setting uses the existing
  settings table). No change to members, payments, or auth.
