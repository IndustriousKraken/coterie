# Coterie

Status: Active Development, alpha.

## Quick Start

```bash
# First-time setup (downloads Tailwind CLI, builds CSS)
make setup

# Copy and configure environment
cp .env.example .env  # then edit .env with your values

# Seed the database with test data (optional, clears existing data)
make seed

# Run the server
make dev
```

Run `make help` to see all available targets.

**Server runs at**: http://127.0.0.1:8080

### Accessing the System

Coterie serves both a **web portal** (for browsers) and a **JSON API** (for integrations).

| Access Method | What You Get |
|---------------|--------------|
| Browser → `http://127.0.0.1:8080/` | Redirects to login page |
| `curl http://127.0.0.1:8080/` | JSON with API endpoint listing |
| `curl http://127.0.0.1:8080/health` | Health check JSON |

### Web Portal Routes

| Route | Description |
|-------|-------------|
| `/login` | Login page |
| `/portal/dashboard` | Member dashboard |
| `/portal/profile` | Edit profile, change password |
| `/portal/events` | View and RSVP to events |
| `/portal/payments` | Payment history |
| `/portal/admin/members` | Admin: manage members |

### API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Health check |
| `GET /api` | API info |
| `GET /public/events` | Public events (JSON or iCal) |
| `GET /public/announcements` | Public announcements |
| `GET /public/feed/rss` | RSS feed |
| `GET /public/feed/calendar` | iCal calendar feed |
| `POST /public/signup` | Register new member |
| `GET/POST /api/members` | Member management (auth required) |
| `GET/POST /api/events` | Event management (auth required) |
| `GET/POST /api/payments` | Payment management (auth required) |

### Test Credentials (Development)

| User | Email | Password | Role |
|------|-------|----------|------|
| Admin | admin@coterie.local | admin123 | Admin |
| Alice | alice@example.com | password123 | Active member |
| Bob | bob@example.com | password123 | Active student |
| Charlie | charlie@example.com | password123 | Expired |
| Dave | dave@example.com | password123 | Pending |

### Stripe Payments (Local Testing)

To test Stripe payments locally, add your test keys to `.env` (gitignored):

```
COTERIE__STRIPE__ENABLED=true
COTERIE__STRIPE__SECRET_KEY=sk_test_...
COTERIE__STRIPE__WEBHOOK_SECRET=whsec_...
```

Stripe sends payment confirmations via webhooks, which can't reach `localhost` directly. The [Stripe CLI](https://docs.stripe.com/stripe-cli) bridges this gap by tunneling webhook events to your local server. In a separate terminal:

```bash
stripe listen --forward-to localhost:8080/api/payments/webhook/stripe
```

This prints a webhook signing secret (`whsec_...`) — use that as your `WEBHOOK_SECRET` above. Leave it running while you test the checkout flow.

On a deployed server with a public URL, webhooks are registered in the Stripe dashboard instead and the CLI isn't needed.

---

Coterie is a secure, lightweight member management system designed for small to medium-sized groups, clubs, and organizations. Built with security and maintainability in mind, it provides a simple yet powerful platform for managing memberships without the complexity of enterprise solutions.

## Overview

Coterie is a member management system for clubs, groups, social organizations etc. 
You can connect it to your website to verify dues payments and register new members, 
and for members to self-service their accounts. Admins can use Coterie to check 
member status, activate/approve memberships, and update member details.

At its core, Coterie strives to do one thing very well: to make sure you know who is in your group, and who is not.

## Architecture

Coterie uses a **dual-frontend architecture** to separate public-facing content from member management:

```
┌─────────────────────┐         ┌──────────────────────┐
│  Public Website     │         │  Management Portal   │
│  (Static Site)      │         │  (HTMX + Alpine.js)  │
├─────────────────────┤         ├──────────────────────┤
│ • Marketing pages   │         │ • Member dashboard   │
│ • Event calendar    │         │ • Admin panel        │
│ • Announcements     │         │ • Payment management │
│ • Signup form       │         │ • Profile editing    │
│ • Member directory  │         │ • Event RSVP         │
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

- **Public Website**: Your existing website (built with any technology) consumes Coterie's public APIs to display events, announcements, and handle signups
- **Management Portal**: Built-in admin and member interface served by Coterie for account management
- **Coterie Backend**: Single Rust binary providing both public and authenticated APIs

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed integration examples.

## Technology Stack

- **Backend**: Rust (using Axum web framework)
- **Database**: SQLite with WAL mode
- **Management Portal**: HTMX + Alpine.js for minimal, secure interfaces
- **Public Website**: Any static site generator or framework (your choice)
- **Authentication**: Session-based with secure cookies, Argon2id for password hashing, TOTP for 2FA
- **Deployment**: Single binary deployment with Caddy reverse proxy

## Core Features

### Built
- **Member Management**: Active / Honorary / Expired / Suspended / Pending statuses; admin CRUD and bulk operations.
- **Payment Integration**: Stripe Elements for one-time and saved-card payments. Coterie-managed auto-renew via scheduled charges; legacy Stripe-managed subscriptions still supported during migration. Donations with optional campaign attribution. Refund flow with idempotency.
- **Public API**: Signup, public events (JSON + iCal), public announcements (JSON + RSS).
- **Admin Dashboard**: Member management, event/announcement editors, manual payment + waive + refund + dues adjustment, audit log viewer, configurable type management (event types, announcement types, membership types), settings UI.
- **Calendar System**: Events with public/member-only visibility, RSVP tracking, configurable event types.
- **RSS / iCal Feeds**: Public announcements as RSS; events as iCal.
- **Audit Logging**: Every admin action recorded with before/after; retention configurable.
- **Email**: Dues reminders, payment-failure notifications, password reset, AdminAlert routing for operational events.

### Integrations
- **Discord**: Member role sync based on dues status, expired-member role handling, daily reconcile cron, AdminAlert email backup when Discord is unreachable.
- **Unifi Access**: API client wired up; access provisioning / revocation flow not yet built.

### Not Yet Built
- TOTP/2FA
- Member-facing receipt downloads (separates dues from donations for tax filing)
- Public donation API endpoint (frontend hosts the form; POSTs to Coterie)
- Member directory (opt-in)
- Recurring donations
- Recurring events (daily/weekly/monthly patterns, custom rules)
- Discord push for announcement publish
- Calendar two-way sync (Google, O365, CalDAV)
- Expense tracking + transparency reports
- Skills directory, achievement badges, voting/polls

See [TODO.md](TODO.md) for the full open-items list.

## Deployment

Production deployment artifacts and walkthroughs live in
[`deploy/`](deploy/):

- [`DEPLOY-DIGITALOCEAN.md`](deploy/DEPLOY-DIGITALOCEAN.md) —
  end-to-end DO droplet (Ubuntu, ~45 min)
- [`DEPLOY-AWS.md`](deploy/DEPLOY-AWS.md) — EC2 + EBS or Lightsail (Ubuntu)
- [`DEPLOY-ALPINE.md`](deploy/DEPLOY-ALPINE.md) — Alpine Linux + OpenRC
- [`MIGRATION.md`](deploy/MIGRATION.md) — moving between hosts
- [`RESTORE.md`](deploy/RESTORE.md) — restoring from a backup
- [`OPS.md`](deploy/OPS.md) — operational reference (secret rotation,
  logs, upgrades, routine maintenance)

A multi-stage [`Dockerfile`](Dockerfile) is provided for container
deploys; the daily backup script + systemd timer
([`deploy/backup.sh`](deploy/backup.sh),
[`coterie-backup.timer`](deploy/coterie-backup.timer)) handles
SQLite snapshots and optional S3-compatible offsite copies.
