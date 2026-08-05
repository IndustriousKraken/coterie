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

- [x] `Dockerfile` for the Coterie binary (multi-stage, slim runtime)
- [x] `systemd.service` template (`deploy/coterie.service`)
- [x] `Caddyfile` example with TLS auto, reverse proxy, security
      headers, gzip (`deploy/Caddyfile.example`)
- [x] `.env.example` annotated with every required setting and what
      breaks if it's missing
- [x] Deploy guide for **DigitalOcean** (`docs/deploy/DEPLOY-DIGITALOCEAN.md`)
- [x] Deploy guide for **AWS** (`docs/deploy/DEPLOY-AWS.md`)
- [x] Deploy guide for **Alpine Linux** (`docs/deploy/DEPLOY-ALPINE.md` —
      OpenRC + crond, fully musl-static, no Docker required)
- [x] Migration runbook: DO ↔ AWS (`docs/deploy/MIGRATION.md`)

### 1.2 Backup
- [x] SQLite `VACUUM INTO` to a timestamped file (`deploy/backup.sh`)
- [x] Daily cron with retention 7 daily + 4 weekly + 12 monthly
      (`deploy/coterie-backup.{service,timer}`)
- [x] Offsite copy hook with S3-compatible defaults; operator picks
      the bucket / provider (env-driven via `COTERIE_BACKUP_S3_URI`)
- [x] Documented restore procedure (`docs/deploy/RESTORE.md`) — needs one
      live test on a throwaway droplet before declaring 1.2 done
      end-to-end



---

## Tier 2 — Launch-adjacent

Build before launch if time permits; otherwise in the first weeks
after. Confirmed priorities for this tier.

### 2.1 Public donation API endpoint
- [x] `POST /public/donate` accepting `{ amount_cents, email, name,
      campaign_slug? }`. Validates amount (positive, ≤ MAX_PAYMENT_CENTS),
      validates campaign is active if present.
- [x] Looks up existing member by email; if found, attaches donation
      to that member. If no match, donation gets recorded with
      donor_name + donor_email on the payment row (member_id NULL,
      enforced by CHECK constraint). Schema change: migration 016
      relaxed payments.member_id NOT NULL.
- [x] Returns a Stripe Checkout URL the frontend redirects to.
      Webhook flow completes identically to the logged-in donate path.
- [ ] **Frontend form lives on the public site**, not Coterie — to be
      built on the public-site side. (Reason: anyone not logged in
      shouldn't reach the portal at all.)
- [x] Rate-limit by IP using the existing `money_limiter`.

### 2.2 Discord push on announcement publish
- [x] When admin transitions an announcement to Published, dispatch
      to a configured Discord channel via the existing
      IntegrationManager. Fires from both the dedicated publish action
      and the create-with-publish-now path.
- [x] Format: visibility tag + title + first paragraph (with
      char-boundary-safe fallback to ~280 chars) + portal link.
      Unit-tested for emoji walls and CRLF paragraph separators.
- [ ] **Per-announcement channel selector** — deferred.

### 2.3 Member receipt downloads
- [x] Per-payment receipt at `/portal/payments/:id/receipt` —
      standalone print-friendly HTML page with org letterhead, member
      name, payment date, amount, type (Dues vs. Donation), campaign,
      and method. PDF deferred (browser "Save as PDF" works fine for
      now). Standalone styling — no portal nav chrome — to keep the
      printed artifact clean.
- [x] "Tax year" view at `/portal/payments/receipts` — completed
      payments grouped by year of paid_at, with Dues and Donation
      totals split. Refunded / pending / failed payments excluded
      (they'd mislead an accountant). Each line links to the printable
      per-payment receipt. Newest year and newest line first.
- [x] Two new org settings (migration 017): `org.address` (for
      letterhead) and `org.tax_id` (optional EIN line). Both editable
      via the existing admin settings UI, render conditionally on the
      receipt.
- Note for future: launching now covers 2026 payments for early-2027
  tax filing. Target completion was October 2026 — done well ahead.

---

## Tier 3 — Quality-of-life

Pick by mood; nothing here gates anything else. Order is suggestion,
not mandate.

### 3.1 Admin TOTP / 2FA
- [x] `totp-rs` (RFC 6238) for codes, `qrcode` for inline-SVG QR.
      Both pure-Rust, no native deps.
- [x] Available to every member via `/portal/profile/security`.
      Admin-only enforcement is a future toggle — the same opt-in
      flow already covers admins.
- [x] 10 recovery codes per enrollment, argon2-hashed in
      `members.totp_recovery_codes` (JSON), one-time use, displayed
      ONCE on enrollment + on regenerate. Format-tolerant verify
      (case/whitespace/hyphens).
- [x] QR + manual-key enrollment page (HTMX swap, no page reload).
      Secret round-trips via hidden field, only persists once a fresh
      TOTP code verifies.
- [x] Two-step login: POST /login mints a 5-minute `pending_login`
      cookie when 2FA is on, redirects to /login/totp; that page
      accepts a 6-digit code OR a recovery code, atomically consumes
      the pending row, and runs the session-fixation sweep before
      issuing the real session.
- [x] Disable + regenerate-codes both require a current TOTP or
      recovery code — design choice: members already have a session,
      so password-reuse here adds no defense.
- [x] **Admin-mandatory toggle** (`auth.require_totp_for_admins`,
      default off, migration 019). When on, `require_admin_redirect`
      bounces unenrolled admins to `/portal/profile/security?reason=
      admin_totp_required` with a banner explaining the policy.
      Member-side access is unaffected; only admin-route gating
      changes. Operators flip the toggle in the new "Authentication"
      category on the admin settings page once their team has
      enrolled.
- [x] Migration 018: members.totp_secret_encrypted (TEXT, encrypted
      with `SecretCrypto`), totp_enabled_at, totp_recovery_codes;
      pending_logins table.
- [x] 13 integration tests (`tests/totp_test.rs`) covering
      enrollment round-trip, wrong-code rejection, off-window verify,
      disable-clears-everything, recovery-code one-time use,
      regenerate-invalidates-old-set, format normalization, and the
      pending_login lifecycle (consume, find-without-consume, expiry,
      disable-wipe). Plus 8 unit tests in the modules themselves.

### 3.2 Recurring events
- [x] **Storage model**: instance-explosion — each occurrence is a
      real `events` row with `series_id` linking to a new `event_series`
      table. Existing queries (RSVPs, list pages, iCal, search,
      Discord) work unchanged because an occurrence is just an event
      that knows it has siblings.
- [x] **Rule subset** in `src/domain/recurrence.rs`: `WeeklyByDay`
      (every Mon, Mon/Wed/Fri, biweekly, etc.), `MonthlyByDayOfMonth`
      (the 15th — months without that day skip rather than clamp),
      `MonthlyByWeekdayOrdinal` ("2nd Wednesday", "last Friday";
      ordinal -1 = last). `until_date` caps the series.
- [x] **Materialization horizon**: 12 months. `RecurringEventService`
      materializes that depth at series-create time; a daily
      background task in main.rs rolls the window forward so the
      calendar always shows ~a year of meetings without operator
      action.
- [x] **Edit semantics** via the detail-page radio: "Edit just this
      occurrence" (default; updates one row) or "Edit this and all
      future occurrences" (`update_series_occurrences_from`
      propagates title/description/type/visibility/location/capacity/
      RSVP forward; per-row date/time/image stay intact).
- [x] **Cancel/delete semantics** via the danger-zone radio:
      "Cancel just this one" (hard-delete the row), "End the series
      here" (delete future + set `until_date`), or "Delete entire
      series" (cascade kills the series row + every occurrence).
- [x] **Discord push** on series creation only — one announcement
      for "Tuesday Coffee, weekly Tuesdays at 6pm", not 52 separate
      posts. Edit/cancel are silent (per design choice).
- [x] **Migration 020** + 14 unit tests (rule generator, including
      DST/skip-short-month edge cases) + 10 integration tests
      (materialization, horizon-extension idempotency, until_date
      capping, edit-this-and-future scoping, end-series, cascade
      delete).
- Skipped the long tail of RFC 5545 deliberately — these three rule
  kinds cover the real-world use cases we've seen.

### 3.3 Admin billing dashboard
- [x] **Upcoming scheduled payments** (next 30 days) via the existing
      `find_pending_due_before(today + 30)`. Member name + due date +
      amount + status + retry count, sorted ascending by due date.
- [x] **Recent failures** (last 90 days) via new
      `ScheduledPaymentRepository::list_failures_since`. Filters on
      `last_attempt_at` so the window matches when the failure
      actually happened. Shows retry_count + failure_reason.
- [x] **Revenue by month** (last 12 months, dues vs. donations split)
      via new `PaymentRepository::revenue_by_month`. Refunded /
      Pending / Failed rows excluded — what we actually collected.
      Folds the flat `(year, month, type)` buckets into one row per
      month with per-bucket counts and totals plus a combined total.
- [x] **Read-only**: every member-name link goes to that member's
      admin page where the actual remediation actions live (refund,
      retry, set dues, edit auto-renew). The dashboard never mutates.
- [x] Routed at `/portal/admin/billing/dashboard`, linked from the
      admin nav under Settings → "Billing dashboard" (alongside the
      existing "Billing" Stripe-migration page).
- [x] 6 integration tests for the new repo methods covering the
      groupings, exclusions, ordering, and time-window filters.

### 3.4 API documentation
- [x] OpenAPI spec generated via `utoipa` macros on the public
      handlers + DTOs. `ApiDoc` lives at `src/api/docs.rs`; the
      handlers in `src/api/handlers/{root,public,announcements}.rs`
      carry `#[utoipa::path(...)]` annotations and the response/request
      types derive `ToSchema`.
- [x] Swagger UI mounted at `/api/docs` with the raw spec served
      at `/api/docs/openapi.json` (see `src/api/mod.rs`).
- [x] Scope is the public API surface only — signup, donations,
      public event/announcement reads, RSS + iCal feeds, plus the
      root/health/api metadata endpoints. Authenticated portal and
      admin routes are intentionally omitted.
- [x] Smoke test (`tests/openapi_test.rs`, 3 tests) asserts the
      spec compiles, every documented path is present, and every
      registered component schema resolves.
- Primary audience: frontend-site developers consuming public APIs.

### 3.5 Payment-flow integration tests
- [x] **Foundation**: `StripeGateway` trait + `RealStripeGateway`
      (production) + `FakeStripeGateway` (tests) — see
      `src/payments/gateway.rs` and `src/payments/fake_gateway.rs`.
      Trait covers all stripe-rs API surface (~13 methods); fake
      records every call and returns canned responses. Test-utils
      Cargo feature exposes the fake to integration tests.
- [x] First flight of tests in `tests/stripe_gateway_test.rs` (9):
      saved-card-charge happy path + RequiresAction + missing-customer
      bail; refund_payment via pi_/cs_/in_ ID forms + unknown-prefix
      reject + cs_-with-no-intent + Stripe-error propagation. All
      green.
- [x] **Migrate remaining StripeClient methods** off direct
      stripe-rs calls onto the trait. All Stripe SDK access now flows
      through the gateway: checkout creation, customer get-or-create,
      setup intents, list/retrieve/detach payment methods, refund
      resolution (cs_/in_ → pi_), subscription cancel, and
      `handle_charge_refunded`'s pi→cs fallback. The legacy
      `client: Client` field on `StripeClient` is gone; the only
      stripe-rs callsite in `stripe_client.rs` is the static
      `Webhook::construct_event` for signature verification (kept off
      the trait deliberately — see `gateway.rs` doc comment).
- [x] Webhook-flow tests in `tests/stripe_webhook_test.rs` (7):
      PI.succeeded retry holds the per-payment dues-extension claim
      so dues_paid_until doesn't shift on the second run; charge.refunded
      echo on an already-Refunded row is a no-op (no UPDATE);
      charge.refunded against a Completed row flips it to Refunded
      (out-of-band Stripe-dashboard refund); customer.subscription.deleted
      for a migrated (coterie_managed) member is silent — the handler
      doesn't clobber billing_mode back to manual; same event for an
      active stripe_subscription member DOES flip to manual (out-of-band
      cancel via Stripe portal); checkout.session.completed for a
      public donation marks the row Completed, doesn't stamp
      dues_extended_at, and upgrades stripe_payment_id from cs_ → pi_.
      Plus a sanity test that none of the above touch the gateway.
- Architecture review flagged that landing the trait unlocks the
  larger refactors (BillingService split, Gateway/Dispatcher
  extraction) with much lower risk.

---

## Tier 4 — Long tail (when someone asks)

Real features, but no obvious user pulling for them yet. Promote a
specific item if a real org requests it.

- Bot protection for the public `POST /signup` and `POST /donate`
  endpoints (CAPTCHA / Cloudflare Turnstile). These are CSRF-exempt by
  design (cross-origin POSTs from the marketing site) and today rely
  only on per-IP rate limiting; a token check from the marketing site,
  verified server-side, would harden them against fake-account creation
  and card-testing (carding) abuse. Promote if abuse actually shows up.
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

### 4.x Separate the public event projection from the `Event` domain type

**Status:** proposed 2026-08-04, not scheduled. Surfaced while specifying
`a52-upcoming-events-persist-until-end`; not caused by it.

**The problem.** `Event::start_time` and `Event::end_time` are documented
as holding the organizer's naive **local wall-clock** in a `DateTime<Utc>`
container — not a real instant. The true instant is derived at read time
by `Event::start_utc()` / `end_utc()`, which resolve that wall-clock
against the event's frozen IANA zone. That model is deliberate and
correct: it is what keeps a 7 PM event at 7 PM across a government DST
rule change, and it is why nothing lossy is ever written to the database.

The `/public/events` handler breaks the invariant in memory. Before
filtering and serializing, `derive_utc_instants` (`src/api/handlers/
public/mod.rs`) **overwrites both fields in place** with the derived
instant, so downstream JSON and iCal emit correct `…Z` timestamps
without a per-call conversion. From that line onward the struct is still
an `Event`, but its two time fields mean the opposite of what the type's
own documentation says they mean.

**Why it matters.** The type no longer answers one question. Any code
holding an `Event` must know *where it came from* to know whether a time
field is a wall-clock or an instant, and the compiler cannot help —
both are `DateTime<Utc>`. Getting it wrong is silent and costs exactly
one UTC offset: four or five hours for `America/New_York`, and **zero
for a UTC-configured instance**, so the mistake ships green through any
test suite whose fixtures use UTC.

This is not hypothetical. Specifying a52's shared upcoming-ness
predicate as a method on `Event` would have introduced precisely this
double conversion at the `/public/events` filter, immediately below the
`derive_utc_instants` call. It was caught in review and worked around:
a52 defines the predicate over **instants** rather than over `&Event`,
and its tasks explicitly forbid the `Event` wrapper at that one call
site. That workaround is sound but it is a convention, and conventions
are what the next contributor does not know about.

**The proposed fix is smaller than it first appears.** The projection
type already exists: `PublicEvent::from_event(e, base_url)` in the same
module. What is missing is the *ordering*. Today the handler sanitizes
`Event`s, derives instants onto them in place, filters and sorts them,
and only then projects. The derivation happens early because
`generate_ical_feed` takes `&[Event]` and needs real instants for
`DTSTART`/`DTEND`.

So the work is:

- Move the wall-clock→instant derivation **into `PublicEvent::from_event`**,
  where it happens exactly once per event and cannot be applied twice.
- Project first, then filter and sort on `PublicEvent` (which already
  carries `start_time`/`end_time`).
- Change `generate_ical_feed` to consume `&[PublicEvent]` instead of
  `&[Event]` — this is the actual load-bearing edit, and the reason the
  current ordering exists.
- Delete `derive_utc_instants` entirely.

Then `Event` keeps one meaning for its whole lifetime, `start_utc()` is
always the right way to get an instant from it, and the mistake a52 had
to warn against becomes unrepresentable rather than documented.

**Cost and risk.** Confined to `src/api/handlers/public/` plus the feed
generators; no schema change, no canon change to
`public-content-feeds`'s observable contract (the JSON and iCal output
are byte-identical if done correctly, which is what makes it testable).
The surface is exercised by the marketing site, by calendar subscribers,
and by the event share/register links, so it wants integration coverage
asserting output equivalence before and after. Not urgent — there is no
live defect today, only a trap.

**Promote when** someone next touches the public feed's serialization
or adds a second consumer of the projected shape. Doing it as part of
that work is much cheaper than doing it on its own.

---

## Cross-cutting notes

- **Architecture refactors** flagged in the architecture review
  (BillingService split into Renewal/Dues/Notifier services,
  StripeClient → Gateway + WebhookDispatcher, Payment domain
  illegal-states cleanup, and the public event projection in 4.x)
  are **deferred until Tier 3.5 lands**.
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
