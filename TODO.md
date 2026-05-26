# Coterie Development TODO

Items still to do. Completed work lives in git history.

## Authentication

- [x] **TOTP/2FA support** — shipped. UI flow in `src/web/portal/security.rs`
      wired to `TotpService`; `totp-2fa` capability spec captures the contract.

## Member Portal

- [x] **Download receipts** (member-facing) — shipped. Per-payment
      printable HTML at `/portal/payments/:id/receipt`; tax-year
      aggregation at `/portal/payments/receipts` with dues vs.
      donations totals split by year. PDF rendering deferred (browser
      print-to-PDF covers it).

## Public-Facing

- [x] **Public donation API endpoint** (`POST /public/donate`) —
      shipped. The frontend form on neontemple.net (or any public site
      in CORS_ORIGINS) is still TODO on the public-site side.

- [ ] Member directory (opt-in)

## Donations

- [ ] Recurring donations (monthly subscription, separate from dues
      auto-renew). Plan-stripe-billing flagged this as optional —
      promote if any org actually wants it.

## Admin

- [x] **Billing dashboard** — shipped. `GET /portal/admin/billing/dashboard`
      renders upcoming scheduled charges, recent failures, and revenue by
      month (see `src/web/portal/admin/billing.rs:189+`, `admin-billing-dashboard`
      capability spec).
- [x] **Recurring events (core)** — shipped. `RecurringEventService` with
      `Recurrence::{WeeklyByDay, MonthlyByDayOfMonth, MonthlyByWeekdayOrdinal}`
      covers weekly, monthly-day-of-month, and ordinal-weekday (e.g. "2nd
      Wednesday", "last Friday") patterns. 52-week rolling horizon with daily
      roll-forward; `until_date` for end dates; series-level edit affects only
      future occurrences. See `event-admin-service` capability spec.
- [ ] **Recurring events — per-occurrence exceptions**: cancel a single
      occurrence (e.g., this Tuesday's meeting is canceled for the holiday)
      and override a single occurrence (e.g., this Tuesday moves to Wednesday)
      without affecting the rest of the series. Spec'd as `a35-recurring-event-exceptions`.
- [ ] Announcement distribution
  - [x] Push to Discord channel on publish — shipped (single org-level
        channel via discord.announcements_channel_id setting; per-
        announcement override deferred)
  - [x] **Scheduled delivery** (publish now vs. schedule for later) — shipped
        via `a11-scheduled-announcement-publish`.
  - [ ] Support for other chat APIs (Slack, Matrix)

## Integrations

- [ ] Discord command interface (status check, etc.) — low priority,
      bot-style features beyond the existing webhook role-sync.
- [ ] Unifi access provisioning (API client exists; provisioning,
      revocation, sync scheduling not wired up)
- [ ] Calendar sync (Google, O365, CalDAV). iCal feed already
      exposes events read-only; this would be two-way.

## Testing

- [ ] Unit tests for domain logic (light coverage today)
- [ ] API endpoint tests
- [ ] Frontend e2e tests (deferred — see project notes)

## Operations

- [ ] Monitoring and alerting setup
- [ ] CI/CD pipeline (GitHub Actions) — staging-only flow exists in
      `deploy/SETUP.md`; full release pipeline still TBD
- [ ] Pre-commit hooks
- [x] **Full Debian provisioning script** — shipped via `a24-provisioning-wizard`
      as the `coterie-provision` Rust binary + thin `deploy/provision.sh` bash
      bootstrap. One command (`curl ... provision.sh | bash`) takes a fresh
      Debian 13 droplet to "Coterie running with Caddy + TLS, first admin
      created, integrations configured." Different mechanism than the original
      `deploy/provision-debian.sh` plan, same goal. See `provisioning-wizard`
      capability spec.

## Documentation

- [x] **API documentation (OpenAPI/Swagger)** — shipped for the public surface
      via `utoipa` in `src/api/docs.rs`. Portal/admin routes are intentionally
      excluded by design (per the docs.rs module comment) so this is "done for
      the surface that needs it." Add a separate item if internal-route docs
      are ever wanted.
- [ ] Administrator guide
- [ ] Installation guide
- [ ] Contributing guidelines
- [ ] Security policy

## Extended Features (Lowest Priority)

- [ ] Expense Tracking
  - Expense entry, receipt upload, categories, quarterly reports,
    public transparency dashboard
- [ ] Member Features
  - Skills directory, blog aggregation from RSS, achievement badges,
    equipment checkout, voting/polls
- [ ] Communication
  - [x] **Event reminders** — shipped via `a10-event-reminder-emails`.
  - [ ] Welcome emails for new members
  - [ ] Announcement digests (rollup of recent announcements via email)
- [x] **Bulk member import/export** — shipped. Export via
      `a12-bulk-member-csv-export`, import via `a13-bulk-member-csv-import`
      with billing-field columns added by `a20-import-billing-fields`.
- [ ] Custom fields for members
- [ ] Report builder
