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
