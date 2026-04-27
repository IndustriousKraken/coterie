# Coterie Development TODO

Items still to do. Completed work lives in git history.

## Authentication

- [ ] TOTP/2FA support

## Member Portal

- [ ] **Download receipts** (member-facing). PDF or simple HTML print
      view, generated per-payment. Must distinguish dues from donations
      so members can hand the donation receipts to their accountant for
      tax filing — group these separately on the receipt page or in
      filenames (`dues-2026.pdf` vs `donation-2026-04-27.pdf`).

## Public-Facing

- [ ] **Public donation API endpoint** (`POST /public/donate`).
      Coterie shouldn't host a public donation page — anyone not logged
      in shouldn't be reaching the portal at all. The frontend site
      (e.g. `~/Dropbox/code/neontemple.net`) hosts the donation form
      and POSTs amount + email + optional campaign_slug to this
      endpoint. Server flow: validate amount and campaign, look up
      existing member by email or create a lightweight donor record,
      create a Stripe Checkout session, return the URL. Webhook flow
      then completes the same as the logged-in donate path.

- [ ] Member directory (opt-in)

## Donations

- [ ] Recurring donations (monthly subscription, separate from dues
      auto-renew). Plan-stripe-billing flagged this as optional —
      promote if any org actually wants it.

## Admin

- [ ] Billing dashboard: upcoming scheduled payments, recent failures,
      revenue metrics. Plan-stripe-billing called for this; current
      admin/billing.rs is just a settings page.
- [ ] Recurring events (recurrence patterns, custom rules like
      "2nd Wednesday", repeat count or end date, edit single vs.
      future occurrences, cancel single occurrence)
- [ ] Announcement distribution
  - [ ] Push to Discord channel on publish
  - [ ] Scheduled delivery (publish now vs. schedule for later)
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

- [ ] Docker containerization
- [ ] SystemD service files
- [ ] Caddy configuration examples
- [ ] Backup scripts
- [ ] Monitoring and alerting setup
- [ ] CI/CD pipeline (GitHub Actions)
- [ ] Pre-commit hooks

## Documentation

- [ ] API documentation (OpenAPI/Swagger)
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
  - Welcome emails for new members, event reminders, announcement
    digests (current emails are payment-related only)
- [ ] Bulk member import/export
- [ ] Custom fields for members
- [ ] Report builder
- [ ] Multi-tenant support
- [ ] GDPR compliance tools
