# Coterie Roadmap

Tiered priority list of what to build next. See `TODO.md` for the raw
open-items list (this doc orders that list and adds context).

Last reviewed: 2026-04-27.

---

## Tier 1 — Production launch blockers

These must exist before deploying for a real org.

### 1.1 Deployment kit
Goal: a fresh ops person can stand up a new instance from scratch in
under an hour by following one document, without ever having to know
a Coterie internal.

- [ ] `Dockerfile` for the Coterie binary (multi-stage, slim runtime)
- [ ] `systemd.service` template
- [ ] `Caddyfile` example with TLS auto, reverse proxy, security
      headers, gzip
- [ ] `.env.example` annotated with every required setting and what
      breaks if it's missing
- [ ] Deploy guide for **DigitalOcean** (droplet sizing, attached
      block storage for the SQLite file, snapshot strategy)
- [ ] Deploy guide for **AWS** (EC2 + EBS or Lightsail, equivalent
      structure)
- [ ] Migration runbook: "how to move from DO → AWS or vice versa"
      (the whole point of having two deploy guides — capture the
      diff so a future migration is mechanical, not improvisational)

### 1.2 Backup
- [ ] SQLite `VACUUM INTO` to a timestamped file
- [ ] Daily cron with retention (suggested: 7 daily + 4 weekly + 12 monthly)
- [ ] Offsite copy hook with S3-compatible defaults; operator picks
      the bucket / provider
- [ ] Documented restore procedure, end-to-end, tested once on a
      throwaway droplet

### 1.3 Manual e2e pass
- [ ] (rab) End-to-end click-through of every member/admin/auth flow
      against a fresh database, including all payment paths, refunds,
      auto-renew toggle, dues reminders, Discord events. Issues that
      surface here become Tier 1.x sub-items.

---

## Tier 2 — Launch-adjacent

Build before launch if time permits; otherwise in the first weeks
after. Confirmed priorities for this tier.

### 2.1 Public donation API endpoint
- [ ] `POST /public/donate` accepting `{ amount_cents, email, name,
      campaign_slug? }`. Validates amount (positive, ≤ MAX_PAYMENT_CENTS),
      validates campaign is active if present.
- [ ] Looks up existing member by email; if found, attaches donation
      to that member. If not, creates a lightweight donor record
      (member with a `Donor` status or similar — TBD during impl).
- [ ] Returns a Stripe Checkout URL the frontend redirects to.
      Webhook flow then completes identically to the logged-in
      donate path.
- [ ] **Frontend form lives in neontemple.net**, not Coterie. Coterie
      only exposes the API. (Reason: anyone not logged in shouldn't
      reach the portal at all.)
- [ ] Rate-limit by IP using the existing `money_limiter`.

### 2.2 Discord push on announcement publish
- [ ] When admin transitions an announcement to Published, dispatch
      to a configured Discord channel via the existing
      IntegrationManager.
- [ ] Channel selector on the announcement form (or fall back to a
      settings-default channel).
- [ ] Format: title + first paragraph + link back to the portal.
- Highest-leverage feature on this tier for NT specifically: removes
  the "post here AND there" friction every single announcement.

### 2.3 Member receipt downloads
- [ ] Per-payment receipt: HTML page styled for print, with org
      letterhead, member name, payment date, amount, type (Dues vs.
      Donation), campaign if any. PDF can come later.
- [ ] "Tax year" view: list of receipts grouped by year, with
      Dues and Donation totals separated. Members hand the donation
      total to their accountant.
- [ ] Time-sensitive: launching now means 2026 payments will be
      wanted for early-2027 tax filing. Target completion **before
      October 2026** so members have a chance to find/use it.

---

## Tier 3 — Quality-of-life

Pick by mood; nothing here gates anything else. Order is suggestion,
not mandate.

### 3.1 Admin TOTP / 2FA
- [ ] `totp-rs` (or equivalent), TOTP-based 2FA
- [ ] Admins first. Members can opt in via profile settings.
- [ ] Recovery codes (mandatory, generated at enrollment, displayed
      once)
- [ ] QR-code enrollment page
- [ ] Two-step login UX for TOTP-enabled accounts
- Rationale: highest-impact compromise is an admin session →
  protect that first.

### 3.2 Recurring events
- [ ] RRULE subset that covers the patterns NT actually uses:
      weekly-by-day, monthly-by-weekday-ordinal ("2nd Wednesday"),
      monthly-by-day-of-month
- [ ] Edit-single-occurrence vs. edit-all-future
- [ ] Cancel-single-occurrence without dropping the series
- Skip the long tail of RFC 5545 — full RRULE support is months of
  work for negligible additional value.

### 3.3 Admin billing dashboard
- [ ] Upcoming scheduled payments (next 30 days)
- [ ] Recent failures (last 90 days, with retry status)
- [ ] Revenue by month, dues vs. donations split
- All read-only. Actions stay on the per-member page.

### 3.4 API documentation
- [ ] OpenAPI spec, auto-generated from handlers if feasible
      (`utoipa` is the obvious choice for Axum)
- [ ] Swagger / Redoc UI at `/api/docs`
- Primary audience: frontend-site developers consuming public APIs.

### 3.5 Payment-flow integration tests
- [ ] Faked `StripeGateway` trait so handlers can be exercised
      without the real Stripe API
- [ ] Suite covering saved-card charge (sync + webhook self-heal),
      refund (admin + dashboard echo), donation (campaign + general),
      Stripe→Coterie migration, dues extension idempotency
- Architecture review flagged that landing this unlocks the larger
  refactors (BillingService split, Gateway/Dispatcher extraction)
  with much lower risk.

---

## Tier 4 — Long tail (when someone asks)

Real features, but no obvious user pulling for them yet. Promote a
specific item if a real org requests it.

- Calendar two-way sync (Google, O365, CalDAV)
- Unifi access provisioning (API client exists; provisioning logic
  doesn't)
- Recurring donations (monthly subscription, separate from dues)
- Member directory (opt-in skills/expertise)
- Discord command interface (status checks, lookups)
- Expense tracking + transparency reports
- Skills directory, achievement badges, voting/polls
- Bulk member import/export
- Welcome emails / event reminders / announcement digests
- Custom fields for members
- Report builder

---

## Cross-cutting notes

- **Architecture refactors** flagged in the architecture review
  (BillingService split into Renewal/Dues/Notifier services,
  StripeClient → Gateway + WebhookDispatcher, Payment domain
  illegal-states cleanup) are **deferred until Tier 3.5 lands**.
  Larger refactors without integration tests are riskier than
  the refactors are worth.
- **Frontend e2e testing agent**: deferred. Prior art exists on
  another project to copy when ready.
- **GDPR compliance tools**: explicitly out of scope. Posture is
  to avoid anything under EU regulatory purview.
- **Multi-tenant**: explicitly out of scope. Each tenant runs as a
  separate instance with its own database. Revisit only if market
  demand makes the operational cost of N instances clearly worse
  than the engineering cost of true multi-tenancy.
