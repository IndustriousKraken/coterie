# Coterie Development TODO

Items still to do. Completed work lives in git history.

## Authentication

- [ ] TOTP/2FA support

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

- [ ] Billing dashboard: upcoming scheduled payments, recent failures,
      revenue metrics. Plan-stripe-billing called for this; current
      admin/billing.rs is just a settings page.
- [ ] Recurring events (recurrence patterns, custom rules like
      "2nd Wednesday", repeat count or end date, edit single vs.
      future occurrences, cancel single occurrence)
- [ ] Announcement distribution
  - [x] Push to Discord channel on publish — shipped (single org-level
        channel via discord.announcements_channel_id setting; per-
        announcement override deferred)
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

- [ ] Monitoring and alerting setup
- [ ] CI/CD pipeline (GitHub Actions) — staging-only flow exists in
      `deploy/SETUP.md`; full release pipeline still TBD
- [ ] Pre-commit hooks
- [ ] **Full Debian provisioning script** — wrap the manual steps in
      `DEPLOY-DIGITALOCEAN.md` into a single idempotent
      `deploy/provision-debian.sh` that takes a fresh droplet from
      bare to "Coterie running with Caddy + TLS" without manual
      intervention. Steps to automate:
      - `apt-get update && apt-get install -y` the prereqs (curl,
        python3, tar, sqlite3, caddy, awscli, ca-certificates)
      - Mount the attached block volume + add the fstab entry
        (detect the by-id path; cope with the volume already being
        mounted if re-run)
      - Run `release-deploy.sh` (which itself runs `install.sh`)
      - Drop the Caddyfile in place from `Caddyfile.example` with
        domain substitution + `mkdir -p /var/log/caddy &&
        chown -R caddy:caddy /var/log/caddy` so a botched first run
        doesn't leave root-owned log files behind
      - Stage `.env` from `.env.example` with `openssl rand -hex 32`
        already substituted into `SESSION_SECRET`
      - Print a "still-to-do" summary at the end (Stripe keys,
        Discord, DNS, `/setup` wizard)

      Goal: a single command turns a fresh droplet into a running
      Coterie. Critical for disaster recovery and for the per-org
      provisioning model if Coterie ever becomes a hosted product.
      Manual deploy is fine the first time; the second time it's a
      chore worth scripting.

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
