# Coterie

Coterie is a secure, lightweight member-management system for small-to-medium
clubs, groups, and organizations. It is the source of truth for who is in your
group and who is not: it tracks membership status, collects dues and donations
through Stripe, manages events and RSVPs, publishes announcements, and gives
members a self-service portal — without the complexity of enterprise software.

It runs as a single Rust binary backed by SQLite, serves its own admin and
member portal, and exposes a small public API your existing website can call
for signups, events, and announcements.

## Deploy

On a fresh Debian 13 host, as root:

```bash
curl -sfL https://raw.githubusercontent.com/IndustriousKraken/coterie/master/deploy/provision.sh \
    -o /tmp/provision.sh
sudo bash /tmp/provision.sh
```

The wizard prompts for your org name, portal domain, first admin credentials,
and which integrations to enable (Stripe / Discord / UniFi / Caddy), then leaves
Coterie running under systemd with the first admin created and (optionally)
Caddy serving with TLS. It is idempotent — re-running it detects existing state
and prompts before overwriting.

For unattended installs (IaC / CI), every prompt has a matching
`COTERIE_PROVISION_<NAME>` env var and `--flag`. Run
`coterie-provision install --help` after the bootstrap downloads the binary, or
read the source at [`deploy/coterie-provision/`](deploy/coterie-provision/).

Deploying somewhere other than Debian, or prefer to drive each step yourself?
See the [deployment guides](#deployment-guides) below.

## Run locally (development)

```bash
# First-time setup (downloads Tailwind CLI, builds CSS)
make setup

# Copy and configure environment
cp .env.example .env   # then edit .env with your values

# Seed the database with test data (optional, clears existing data)
make seed

# Run the server
make dev
```

The server runs at <http://127.0.0.1:8080>. Run `make help` to see all targets
and `cargo test --features test-utils` to run the full suite.

### Test credentials (seeded data)

| User | Email | Password | Role |
|------|-------|----------|------|
| Admin | admin@coterie.local | admin123 | Admin |
| Alice | alice@example.com | password123 | Active member |
| Bob | bob@example.com | password123 | Active student |
| Charlie | charlie@example.com | password123 | Expired |
| Dave | dave@example.com | password123 | Pending |

### Testing Stripe locally

Add your Stripe test keys to `.env` (gitignored):

```
COTERIE__STRIPE__ENABLED=true
COTERIE__STRIPE__SECRET_KEY=sk_test_...
COTERIE__STRIPE__WEBHOOK_SECRET=whsec_...
```

Stripe delivers payment confirmations by webhook, which can't reach `localhost`
directly. The [Stripe CLI](https://docs.stripe.com/stripe-cli) tunnels them to
your local server — in a separate terminal:

```bash
stripe listen --forward-to localhost:8080/api/payments/webhook/stripe
```

It prints a signing secret (`whsec_...`); use that as `WEBHOOK_SECRET` above and
leave it running while you test checkout. On a deployed server with a public URL
you register the webhook in the Stripe dashboard instead and the CLI isn't
needed. Full walkthrough: [`deploy/STRIPE-SETUP.md`](deploy/STRIPE-SETUP.md).

## How it works

Coterie uses a dual-frontend architecture that separates your public website
from member management:

```
┌─────────────────────┐         ┌──────────────────────┐
│  Public Website     │         │  Management Portal   │
│  (your static site) │         │  (HTMX + Alpine.js)  │
├─────────────────────┤         ├──────────────────────┤
│ • Marketing pages   │         │ • Member dashboard   │
│ • Event calendar    │         │ • Admin panel        │
│ • Announcements     │         │ • Payment management │
│ • Signup form       │         │ • Profile editing    │
└──────────┬──────────┘         └──────────┬───────────┘
           │                                │
           ▼                                ▼
     Public APIs                     Protected APIs
           │                                │
           └────────────┬───────────────────┘
                        │
                 ┌──────▼──────┐
                 │   Coterie   │
                 │   Backend   │
                 └─────────────┘
```

- **Public website** — your existing site (any technology) calls Coterie's
  public APIs to show events and announcements and to accept signups.
- **Management portal** — the admin and member interface, server-rendered with
  HTMX and served by Coterie itself. This is the only admin surface.
- **Coterie backend** — a single Rust binary serving both, backed by SQLite.

See [ARCHITECTURE.md](ARCHITECTURE.md) for integration examples and the
surface-by-surface security model.

**Stack**: Rust (Axum) · SQLite (WAL) · HTMX + Alpine.js portal · session auth
with Argon2id + TOTP 2FA · single-binary deploy behind Caddy.

## Features

- **Member management** — Active / Honorary / Expired / Suspended / Pending
  statuses; admin CRUD; bulk CSV import and export.
- **Payments** — Stripe Elements for one-time and saved-card payments;
  Coterie-managed auto-renew via scheduled charges (legacy Stripe-managed
  subscriptions still supported during migration); donations with optional
  campaign attribution; refunds with idempotency; member-facing receipts and
  tax-year summaries.
- **Events** — public / member-only visibility, RSVP tracking, configurable
  types, recurring series (weekly, monthly-by-date, ordinal-weekday) with
  single-occurrence edits and cancellations, iCal feed, and email reminders.
- **Announcements** — publish now or schedule for later; RSS feed; Discord push
  on publish.
- **Admin dashboard** — member / event / announcement editors; manual payment,
  waive, refund, and dues adjustment; billing dashboard; audit-log viewer;
  configurable types; settings UI.
- **Public API** — signup, donations, public event and announcement reads, RSS
  and iCal feeds; documented as OpenAPI at `/api/docs`.
- **Security** — session cookies (HttpOnly / Secure / SameSite); top-level CSRF;
  per-IP rate limiting on auth and money-moving endpoints; full audit logging;
  optional admin-mandatory 2FA.
- **Integrations** — Discord role sync by dues status (with email fallback);
  UniFi Access API client wired up.

See [ROADMAP.md](ROADMAP.md) and [TODO.md](TODO.md) for what's planned next.

## Routes & endpoints

Browsers hitting `/` are redirected to login; `curl /` returns a JSON endpoint
listing; `GET /health` is the health check.

**Portal** (`/portal/*`, server-rendered):

| Route | Description |
|-------|-------------|
| `/login` | Login |
| `/portal/dashboard` | Member dashboard |
| `/portal/profile` | Profile, password, 2FA |
| `/portal/events` | View and RSVP to events |
| `/portal/payments` | Payment history and receipts |
| `/portal/admin/*` | Admin: members, events, announcements, payments, settings, audit |

**Public API** (`/public/*`, for your website):

| Endpoint | Description |
|----------|-------------|
| `POST /public/signup` | Register a new member |
| `POST /public/donate` | One-time donation (returns a Stripe Checkout URL) |
| `GET /public/events` | Public events (JSON or iCal) |
| `GET /public/announcements` | Public announcements |
| `GET /public/feed/rss` | RSS feed |
| `GET /public/feed/calendar` | iCal calendar feed |

`/api/*` is intentionally narrow — just the Stripe webhook and the saved-card
(Stripe.js) endpoints. Admin actions live exclusively in the portal; see
[CLAUDE.md](CLAUDE.md) for the rationale.

## Deployment guides

For operators who want to drive each step, deploy on another platform, or run
day-to-day operations:

- [`DEPLOY-DIGITALOCEAN.md`](deploy/DEPLOY-DIGITALOCEAN.md) — end-to-end DO
  droplet (Ubuntu, ~45 min)
- [`DEPLOY-AWS.md`](deploy/DEPLOY-AWS.md) — EC2 + EBS or Lightsail (Ubuntu)
- [`DEPLOY-ALPINE.md`](deploy/DEPLOY-ALPINE.md) — Alpine Linux + OpenRC (no
  Docker required)
- [`STRIPE-SETUP.md`](deploy/STRIPE-SETUP.md) — wiring Coterie to a Stripe account
- [`OPS.md`](deploy/OPS.md) — operations reference (secret rotation, logs,
  upgrades, routine maintenance)
- [`MIGRATION.md`](deploy/MIGRATION.md) — moving between hosts
- [`RESTORE.md`](deploy/RESTORE.md) — restoring from a backup

A multi-stage [`Dockerfile`](Dockerfile) is provided for container deploys; the
daily backup script and systemd timer ([`deploy/backup.sh`](deploy/backup.sh),
[`coterie-backup.timer`](deploy/coterie-backup.timer)) handle SQLite snapshots
and optional S3-compatible offsite copies.
