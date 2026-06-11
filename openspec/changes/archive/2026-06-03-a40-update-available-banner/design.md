## Context

a39 makes the running release version known in-process (`crate::version`). a38
makes updating a one-liner. The missing piece is the *prompt*: an operator on an
older release currently has to remember to check GitHub. This change adds a
background check + an admin banner, reusing the established patterns — a
`tokio::spawn` daily loop (like the Discord reconcile and recurring-event
horizon jobs) and a cached value on `AppState` read at render time. It depends on
a39 and ships after it.

The guiding constraint is the resilience rule already in `docs/ARCHITECTURE.md`: no
outbound network call in the request path. The banner reads a cached value; only
the background task talks to GitHub.

## Goals / Non-Goals

**Goals:**
- Admins running an older **stable** release see a clear, dismissible prompt with
  links to the release notes and update steps.
- The check is background-only, cached, opt-out, and silent when GitHub is down.
- Version comparison is a pure, unit-tested function that ignores prereleases.

**Non-Goals:**
- **Auto-updating.** Applying the update is a38 / an operator action; this only
  notifies.
- **Notifying members.** The banner is admin-only.
- **Prerelease / dev-build nagging.** No banner for `-rc`/`-dev` releases, and
  none at all for builds with no embedded release tag.
- **Server-side per-admin dismissal state.** Dismissal is a cookie; no schema.
- **An in-portal release-notes reader.** The banner links out to GitHub.

## Decisions

### D1. Background daily check, never in the request path

A `tokio::spawn` task (initial short delay, then every 24h — mirroring the
reconcile/horizon jobs in `main.rs`) fetches `releases` from the GitHub API,
selects the latest non-prerelease, and stores it in a cache on `AppState`. The
banner render reads only the cache, so page latency is never coupled to GitHub.

### D2. In-memory cache on `AppState`, no schema change

The cached latest-stable release (tag + html_url, or `None`) lives in an
`Arc<RwLock<Option<…>>>` on `AppState`, with a matching `FromRef` impl per the
convention noted in `state.rs`. It is refreshed by the task and lost on restart
(re-fetched on the next cycle, after the initial delay). No migration, no
settings row for the cached value.

### D3. Only nag release builds

The comparison runs only when `crate::version::release_tag()` is `Some` — i.e. a
real release build. Development/untagged builds never show the banner (there is
nothing meaningful to compare, and the dev is presumably the maintainer).

### D4. Stable-only + pure comparator

Prereleases are excluded using the GitHub `prerelease` flag (the same flag a38's
"latest stable" resolution keys off). Among stable tags, a pure
`is_newer(latest, running) -> bool` compares `vX.Y.Z` (leading `v` stripped);
unparseable tags are ignored rather than erroring. The comparator takes its
inputs as arguments so it is fully unit-testable.

### D5. Admin-only render

The banner is rendered only for admins (the same gate the admin nav uses). A
member session never sees it — they cannot update the server, so it would be
noise.

### D6. Dismiss until a newer version ships

A dismiss control sets an `update_dismissed=<latest-tag>` cookie; the banner is
suppressed while the dismissed tag equals the current latest-known. When a newer
release appears, the dismissed tag no longer matches and the banner returns.
Cookie-based so there is no new route, CSRF surface, or schema; the render simply
honors the cookie.

### D7. Opt-out setting, default on

`updates.check_enabled` (boolean, default `true`) gates the whole feature: when
off, the task performs no fetch and the banner never renders. Default-on is the
right call for a security-sensitive app, but it does mean the instance contacts
`api.github.com` (unauthenticated, low-information). The setting gives operators
who object a clean off switch, surfaced in the admin settings UI.

### D8. Graceful degradation

A GitHub error (network, rate-limit, parse) leaves the cache unchanged and logs
at warn/debug — never an error banner, never a failed page. The worst case is a
stale-by-a-day or absent "update available" hint, which is harmless.

## Risks / Trade-offs

- **Phoning home.** Mitigated by the opt-out (D7) and by sending nothing but an
  unauthenticated GET to the public releases API.
- **GitHub rate limits.** A daily unauthenticated check is far under the 60/hr
  limit; the cache means render never calls out.
- **Comparator edge cases** (odd tags, missing minor/patch). Mitigated by
  ignoring unparseable tags and unit-testing the comparator directly.

## Migration Plan

Single change, after a39:

1. Add the pure version comparator + unit tests.
2. Add the release fetch + latest-stable selection (using `reqwest`).
3. Add the cache field + `FromRef` on `AppState`.
4. Spawn the daily check task in `main.rs` (initial delay, then 24h), reading
   `updates.check_enabled` each cycle.
5. Add the `updates.check_enabled` setting (default on) and surface it in admin
   settings.
6. Add the admin-gated banner + dismiss control to `base.html`, linking to the
   release notes and the README "Update" steps.
7. Tests: comparator unit tests; render tests (admin + newer → shown; member →
   hidden; dev build → hidden; dismissed-cookie matches latest → hidden).
8. `cargo build` / `test` / `clippy --deny warnings` / `fmt --check` /
   `openspec validate`.
