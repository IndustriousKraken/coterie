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

- `coterie-provision` now ships in the main release tarball and is installed to
  `/opt/coterie/coterie-provision` by both the first install and every update,
  so the hardened update path `release-deploy.sh` delegates to is actually on
  the box instead of being downloaded on every run.

### Changed

### Fixed

- `release-deploy.sh` no longer runs `deploy/update.sh` through `sh`. `update.sh`
  is bash and `/bin/sh` is dash on Debian, so an update on an instance without
  `coterie-provision` died with `trap: 26: bad trap` and deployed nothing. The
  script is now exec'd directly so its shebang chooses the interpreter, and
  falling back to that bootstrap is announced instead of silent.

## [v1.0.20] — 2026-07-31

### Added

- Adds a backup that actually protects a deployment — a run now bundles the vacuumed database together with both upload roots into one timestamped archive, a new `deploy/restore.sh` verifies a bundle and restores it (stopping the service, moving current state aside rather than deleting it, fixing ownership, and integrity-checking the database), and provisioning installs and enables the backup timer so a fresh install is protected by default. Existing deployments still need the units installed once; the deploy docs and `RESTORE.md` — which now covers uploads — carry the step.

### Changed

- Splits four oversized source units — the public API handlers, the duplicated event create/update multipart parsing, the Stripe JSON test fixtures, and `coterie-provision`'s installer — with no change to any route, the OpenAPI document, the CLI surface, or a test assertion. The single visible effect: the admin event-edit form now reports an invalid start time with the same alert the create form uses.

### Security

- Refuses member class enrollment on an `AdminOnly` class. `POST /portal/api/series/:id/enroll` was the one member registration endpoint the previous `AdminOnly` fix missed, so posting a series id directly seated the member on every future admin-only occurrence and, on a priced class, disclosed its title to them on a Stripe checkout page; it now answers with the same "Class not found" fragment an unknown id produces. An org that expects members to enroll in a recurring class should set that series to `MembersOnly`.

## [v1.0.19] — 2026-07-31

### Security

- Stores private submission attachments under a separate uploads root that no public route is mounted on, so `GET /uploads/:filename` cannot reach an attachment regardless of what the database says. The previous denylist failed open — an attachment became public whenever its row went away (replaced on edit, submitter deleted, a failed cleanup, or the window between writing the file and committing the row). Existing attachments are migrated, and member-only event images now fail closed too: the route asks whether a file is known public rather than whether it is known private.
- Neutralizes spreadsheet formula injection in the finance tax-prep CSV export. Its description, counterparty, category, and account columns carry attacker-supplied text — a public donor's name and email, or any member's profile name — and are now written with the same formula-neutralizing writer the member roster and audit-log exports already use.

## [v1.0.18] — 2026-07-31

### Added

- Adds security logging across the authentication surface — failed and successful logins, TOTP outcomes, password-reset requests and completions, password-policy rejections, and rate-limit trips now emit structured log events, so "why couldn't this member log in?" is answerable. Account-state changes (password changed or reset, TOTP enabled or disabled, recovery codes regenerated) also write audit-log rows. The log distinguishes unknown email from wrong password from inactive account; the HTTP response does not. Passwords, reset tokens, session tokens, and TOTP codes are never logged.
- Adds an optional `org.signup_url` org setting. The portal login page's "create account" link now renders only when it is set, and points at the organization's own join page; previously it pointed at a POST-only JSON API, so following it downloaded a 405 error body as a file.

### Changed

- States the maximum password length in bytes rather than characters, and reports what was submitted — a non-ASCII password was previously refused for exceeding a character count it had not exceeded. The enforced ceiling is unchanged; password fields now show it before submission rather than after.

### Fixed

- Fixes failed logins locking a member out of password recovery. `/forgot-password` now has its own rate-limit budget instead of sharing the login budget, so five wrong passwords no longer block the reset that fixes them. The TOTP second factor still shares the credential budget.
- Fixes `POST /reset-password` returning `200` for a rejected reset — a bad, expired, or already-used token, or a password failing policy — which made a refused reset indistinguishable from a successful one in logs and monitoring. The rendered page is unchanged.
- Fixes the portal surface producing no request logs at all: the tracing layer was applied before the API and web routers were merged, so portal requests, including every rate-limit rejection, were invisible to the application.
- Fixes a series-wide delete stranding guests' paid registration fees. A series that was free for members but priced for guests passed the refund-before-delete guard, so the delete kept the guests' money and cascaded away the roster recording who was owed it; the refusal is now driven by outstanding completed event-fee payments rather than by the member price column.
- Refuses to sell a class pass when no session of the series remains — the sale previously charged the full pass price and seated the buyer on nothing. Flat pricing is otherwise unchanged, and the admin at-the-door and comp path is deliberately exempt.
- Fixes a horizon-window flake in the recurring-series tests that failed CI only on certain weekdays, and requires horizon-extension tests to derive their bounds from the materializer horizon rather than the wall clock.

### Security

- Makes the first-run setup gate fail closed. The unauthenticated, CSRF-exempt `POST /setup` read a database error on its "has an admin already been created?" check as "this is a fresh install", so anyone who could make that one query fail — by exhausting the connection pool, or during any write-lock contention — could mint a second full administrator on a live instance. An errored check now refuses, as does a process that has already observed an admin.
- Hides `AdminOnly` events from the member portal. Every member could previously see admin-only events in full on the events list and dashboard, and RSVP to them; those surfaces and the RSVP/cancel endpoints now apply the visibility level, and admins still see everything. An org using `AdminOnly` as a scheduling marker will find those events disappear from members' lists; attendance rows already written against them survive on the admin roster.
- Bounds `full_name` on the member profile-update endpoint at 200 characters, trimmed and non-empty, matching public signup. An authenticated member could otherwise store a megabyte-long or blank name, which then rendered on the admin member list, every event and class roster, the member CSV export, and outbound email.

## [v1.0.17] — 2026-07-27

### Added

- Adds paid events — an admin can put a member price on an event, and registering routes through Stripe Checkout instead of a plain RSVP: the seat is claimed before the payment starts, capacity is enforced for paid events, and a refund releases the seat (deleting a paid event refunds every paid attendee first, and refuses to delete if a refund fails). Each paid event gets an admin roster showing payment state, with at-the-door and comped recording; event fees are recorded as their own payment kind, so event revenue is separable in billing.
- Adds public registration for paid events — a separate guest price and a "guest registration may happen at all" toggle, plus a shareable Coterie-hosted registration page at `/events/:id/register` that can be pasted into a post or newsletter. The page is protected by the bot challenge, the money rate limiter, and a public-visibility check, offers members a login for member pricing, and never attaches a guest to an existing member's account; the public events feed now carries a registration URL and guest price for registerable events.
- Adds series passes for paid classes — one payment enrolls an attendee in every remaining session of a bounded recurring series ("six Tuesdays, $120"), with capacity counted across the class rather than per night. Pricing is flat (joining late costs full price, cancelling refunds in full), attendance is materialized per occurrence so rosters, check-in, reminders, and iCal keep working unchanged, and cancelling a single occurrence is not a refund event.

## [v1.0.16] — 2026-07-24

### Added

- Adds bot-challenge (Turnstile) configuration to the admin settings page — provider, secret key (encrypted at rest), and timeout are stored in the database like the Stripe and Discord integrations, so an operator can turn the captcha on or rotate its key from the portal instead of editing a file on the server, and the change takes effect without a restart. The environment-variable configuration is retired.
- Adds owner actions on terminal submissions — a member can delete their own `withdrawn` or `declined` submission (removing its attachment), and re-open a `withdrawn` one back to `submitted` for revision and resubmission; a `declined` submission is not re-openable.

### Changed

- Shows members only settled payments (`Completed` and `Refunded`) in their Payments history, so abandoned checkouts no longer fill it with `Pending`/`Failed` rows. Admin payment views still show every status, and a genuinely failed renewal still surfaces through the dues-status pill and reminder emails.

### Security

- Requires a verified email before payment-mode signup reaches Stripe — signup now creates the pending member and sends the verification email, and the Stripe customer and Checkout session are created only when that link is clicked. This closes the card-testing path where an unauthenticated request ran a stolen card through the org's live Stripe in one hop; the abandoned-checkout retry requires verification too, and legitimate members click one link before paying.

## [v1.0.15] — 2026-07-22

### Added

- Adds an opt-in date range (`from`/`to`) to `GET /public/events`, so the public marketing calendar can show a month's past events; the default response and iCal subscriptions stay upcoming-only.

## [v1.0.14] — 2026-07-22

### Fixed

- Fixes the wrong date showing on the dashboard payment view.

## [v1.0.13] — 2026-07-21

### Security

- Stops the public `/uploads/:filename` route serving submission attachments, so a private submission PDF can no longer be fetched by guessing its filename — it is reachable only through its authorization-gated download route.

## [v1.0.12] — 2026-07-21

### Added

- Adds a "Submissions" link to the admin navigation (shown only when member proposal submissions are enabled), so admins can reach the submission review and promotion page.

## [v1.0.11] — 2026-07-21

### Added

- Adds Markdown rendering for admin announcements — bodies authored in Markdown now render as sanitized HTML in the member portal, the public marketing site, and the RSS feed, making the form's "Markdown formatting is supported" hint true.
- Adds member proposal submissions (off by default) — when enabled, members can submit talk or session proposals with an optional PDF attachment for admin review, and an accepted submission can be promoted to an event; reviewer decisions are audited.

### Security

- Stops `GET /public/announcements` exposing internal announcement fields — the author's member ID, internal timestamps, and scheduling fields — to anonymous callers; the endpoint now returns a purpose-built public projection.

## [v1.0.10] — 2026-07-17

### Fixed

- Fixes the Events page returning an error when "Show past events" is checked; the checkbox value could not be parsed, so ticking the box broke the events list.
- Fixes the dashboard's Upcoming Events widget showing a registered event as a non-interactive "Attending" label, so members can now cancel an RSVP directly from the dashboard.

## [v1.0.9] — 2026-07-16

### Fixed

- Fixes first-run setup reporting success even when activating the initial admin failed — the new admin was left unable to log in and setup could not be retried, locking the organization out; the failure now surfaces instead of being swallowed.
- Fixes "Remember me" breaking login — checking the box made the login request fail to deserialize, so valid credentials were rejected whenever it was ticked.
- Fixes the Announcements page returning an error when "Show all" is checked; the checkbox value could not be parsed into the list filter.
- Fixes the event RSVP/cancel toggle only updating after a manual page refresh, because the first swap detached its HTMX target.

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
