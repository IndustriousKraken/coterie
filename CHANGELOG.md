# Changelog

All notable changes to Coterie are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project aims to follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file is the **source of truth for release bodies**: the release workflow
publishes the section whose header matches the tag being released (see
`.github/workflows/release.yml`). Once a version's tag is published, its section
is **immutable** — fix mistakes in a later version's notes, never by rewriting a
released section. Only `[Unreleased]` is ever ahead of every tag; everything
under a version header is accurate for that version forever.

<!--
Release convention (performed at release prep by a maintainer / the
autocoder's changelog generator, NOT by ordinary changes):

  1. Rename the `## [Unreleased]` header to `## [vX.Y.Z] — YYYY-MM-DD`
     (em-dash, ISO date), keeping its Added/Changed/Fixed entries.
  2. Add a fresh empty `## [Unreleased]` section above it.
  3. Commit, THEN tag that commit `vX.Y.Z` — so the tagged commit carries
     its own finalized entry and the release body is sourced from it.

Sections are ordered newest-first. Use the Keep a Changelog subheads:
Added / Changed / Deprecated / Removed / Fixed / Security.
-->

## [Unreleased]

### Added

### Changed

### Fixed

## [v1.0.8] — 2026-07-14

### Added

- Adds a published security disclosure and bug-bounty policy (`SECURITY.md`), documenting scope, safe harbor, and how to report vulnerabilities.

### Fixed

- Fixes event past/upcoming status being off by the org's UTC offset — an event could show as "past" hours before it actually started on the admin event list and detail, series-occurrence rows, and the member events list; status is now computed from the event's true instant rather than its raw wall-clock time.

## [v1.0.7] — 2026-07-13

### Fixed

- Rejects public signups that target a deactivated membership type.

### Security

- Stops the login and money rate limiters (and the bot-challenge client IP) trusting a spoofable, client-supplied `X-Forwarded-For` value; the client IP is now taken from the trusted proxy hop instead of the left-most header entry, so it can no longer be rotated per request to evade the limiters.
- Closes a timing side-channel in the payment-mode signup retry that let an attacker distinguish a pending-unpaid signup email from other addresses by response latency alone.
- Rate-limits public signup in approval mode (previously rate-limited only in payment mode), curbing mass account creation and verification-email abuse.
- Stops `GET /public/events` from exposing internal fields — the organizer's member ID and internal timestamps — to anonymous callers.

## [v1.0.6] — 2026-07-11

### Added

- Adds org-defined custom member fields, so an org can capture attributes Coterie doesn't model (external profile IDs, a rank, a committee) as structured data instead of free-text notes, without hardcoding columns.

## [v1.0.5] — 2026-07-11

### Added

- Wires up the admin member Quick Actions — the "Send Password Reset" and guarded "Delete Member" buttons now work instead of returning a 404.

## [v1.0.4] — 2026-07-11

### Changed

- Auto-enrolls paid signups in auto-renewal, and reuses the member's open session when they retry a pending signup.

## [v1.0.3] — 2026-07-11

### Added

- Adds pay-at-signup — in payment mode a signup returns a Stripe checkout and activates the member as soon as payment completes, instead of waiting for manual approval.
- Adds a public `GET /public/membership-types` endpoint, so join forms can list the org's real membership types instead of a hardcoded slug that drifts from the database.
- Adds a working administrator toggle to the member-edit form, replacing the non-functional "ADMIN" notes convention that granted no access.
- Adds portal configuration for Stripe and UniFi, so operators can add or rotate them from the admin UI instead of editing `.env` and restarting.
- Adds import of Stripe payment history and saved cards when migrating from an existing billing system.
- Adds a semiannual (6-month) billing period.

### Fixed

- Fixes event times displaying off by the org's UTC offset on the public site and calendar feeds (the admin portal was already correct), and stops evening events being dropped early from upcoming lists.
- Fixes scheduled announcements publishing at the wrong time (the same timezone bug).
- Fixes financial statements bucketing dues by UTC instead of the org's tax year.
- Fixes Stripe subscription cancellation reporting a false "upstream error" when the cancellation actually succeeded.
- Fixes payment lists showing the row-creation date instead of the actual payment date.
- Sets CORS for the configured marketing domain during provisioning, so the marketing site's calls to the public API are no longer discarded by the browser.
- Fixes admin dropdowns not working under the Alpine CSP build.
- Fixes the admin member-edit "Save Changes" button doing nothing inside a nested form.
- Fixes admin settings boolean toggles being unsavable.
- Fixes admin list tables clipping the Actions column on wide rows.
- Fixes the saved-cards list not refreshing after changes, and adds a cancellation confirmation.
- Tolerates partially-configured UniFi and Discord integrations instead of failing.
- Sends the subscription receipt email only after dues are extended.
- Raises the provisioned Caddy request-body limit from 1 MB to 12 MB so larger uploads succeed.

### Security

- Caps the maximum accepted password length.

## [v1.0.2] — 2026-06-30

### Fixed

- Fixes subscription webhooks marking a payment "Completed" before extending the member's dues — a transient failure during dues extension could leave the payment completed but the dues never extended (the Stripe retry short-circuited on the already-completed row); the handler now extends dues first, so a retry recovers.

### Security

- Bounds the length of the public signup email, username, and full-name fields on the unauthenticated signup endpoint.

## [v1.0.1] — 2026-06-26

### Added

- Adds a first-run admin bootstrap CLI (`create_admin`), so the initial administrator can be created from the server instead of racing to claim the unauthenticated `/setup` page after a fresh deploy.
- Adds the `coterie-provision` wizard, which takes a clean Debian host to a running, TLS-terminated Coterie install in one guided pass.
- Adds a Stripe test mode with a guided switch to live keys, so operators can verify payment wiring before taking real charges.
- Hardens the update path (`coterie-provision update`) with a pre-swap database snapshot and a post-restart health check that rolls back a release which fails to come up.
- Adds version awareness — the instance reports its own version and shows admins an "update available" banner when a newer stable release exists.
- Adds per-occurrence exceptions for recurring events — cancel or override a single occurrence without changing the rest of the series.
- Adds expense tracking, so orgs can record outgoing costs alongside income in Coterie.
- Adds audit-log entries for basic-type and membership-type admin changes.

### Fixed

- Fixes the CSRF layer rejecting every browser-facing login, 2FA, password-reset, and first-run-setup POST.
- Fixes a panic during web login when creating the session.
- Fixes panics when truncating multi-byte UTF-8 text in admin announcement and member views.
- Fixes recurring-event occurrence indexes drifting after an occurrence is cancelled, which could misalign later occurrences.
- Fixes three error-handling bugs in the `coterie-provision` wizard.

### Security

- Enforces 2FA on the JSON login endpoint — a TOTP-enrolled member is no longer issued a session without completing the second factor.
- Makes TOTP verification fail closed and rate-limits the second-factor step on both the web and JSON login flows.
- Invalidates a member's other sessions when they change their password.
- Neutralizes spreadsheet formula injection in admin CSV exports.
- Claims scheduled payments atomically, closing a double-charge race.

## [v1.0.0] — 2026-05-21

### Added

- Adds automated event reminder emails — members who RSVP now receive a reminder before each event, with a configurable lead time (`events.reminder_lead_hours`, default 24 hours).
- Adds scheduled announcement publishing — admins can set a future publish time on an announcement, and a background runner publishes it automatically when that time arrives.
- Adds bulk member CSV export — admins can download the full member roster as a CSV file.
- Adds bulk member CSV import — admins can create members in bulk from a CSV, including optional billing-migration columns (paid-through date, Stripe customer and subscription IDs, join and email-verification dates) for onboarding from an existing billing system.
- Adds operator alerts for failed Coterie-managed renewals — a terminal auto-renew charge failure now dispatches an admin alert (email and/or Discord) rather than only writing a log line.

### Changed

- Caches the post-setup "an admin exists" check, eliminating a database query that previously ran on every non-static request.

### Removed

- Removes three unused saved-card JSON endpoints (`GET /api/payments/cards`, `DELETE /api/payments/cards/:id`, `PUT /api/payments/cards/:id/default`); the portal UI and Stripe.js card flows are unaffected.
