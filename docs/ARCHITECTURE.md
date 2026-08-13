# Coterie Architecture

## Overview

Coterie uses a **dual-frontend architecture** to separate concerns between public-facing content and member/admin management functionality.

## System Components

### 1. Coterie Backend (API Server)
**Technology**: Rust with Axum framework  
**Purpose**: Central API server providing all business logic and data management

#### Public APIs (`/public/*`)
- No authentication required
- Designed for consumption by static sites and the marketing site
- Endpoints:
  - `POST /public/signup` - New member registration
  - `POST /public/donate` - One-time donation (creates a Stripe Checkout session)
  - `GET /public/events` - Public event listings (JSON)
  - `GET /public/announcements` - Public announcements (JSON)
  - `GET /public/feed/rss` - RSS feed of announcements
  - `GET /public/feed/calendar` - iCal calendar feed

The full public surface is documented as an OpenAPI spec at
`/api/docs/openapi.json` (see `src/api/docs.rs`). Cross-origin POSTs
from the marketing site are CORS-gated, not CSRF-gated — see
`CLAUDE.md` for the rationale.

#### Narrow JSON surface (`/api/*`)

`/api/*` is intentionally narrow — **not** a CRUD API. It carries:

- `POST /api/payments/webhook/stripe` — inbound Stripe webhook,
  authenticated via Stripe's HMAC signature.
- `/api/payments/cards/*` — saved-card management endpoints called
  directly by the portal frontend's Stripe.js integration (which
  needs JSON in / JSON out for the SetupIntent flow).

There is **no** admin CRUD on members / events / announcements /
payments / settings / types under `/api/*`. Admin actions live
exclusively in the management portal under `/portal/admin/*`. A
parallel JSON admin surface used to exist; it was deleted in 2026-04
because it had drifted into half-strength duplicates that skipped
audit logs and integration events, and was missing CSRF protection.
See `CLAUDE.md` for the rule and the rationale.

### 2. Public Website (Static Site)
**Technology**: Any static site generator (Hugo, Jekyll, Next.js, etc.)  
**Purpose**: Marketing, information, and public engagement

#### Features:
- **Informational Pages**: About, membership benefits, contact
- **Dynamic Content via API**:
  - Event calendar (fetched from `/public/events`)
  - News/announcements (fetched from `/public/announcements`)
  - Member signup form (posts to `/public/signup`)
  - Donation form (posts to `/public/donate`)
- **Feed Integration**:
  - RSS feed for news readers
  - iCal subscription for calendars

#### Deployment:
- Can be hosted anywhere (GitHub Pages, Netlify, Vercel, S3)
- Fetches data client-side or at build time
- No server-side code required

### 3. Management Portal
**Technology**: HTMX + Alpine.js (server-side rendered)  
**Purpose**: Member self-service and admin management

#### Member Features:
- Login/logout
- Profile management
- Event RSVP
- Payment history
- Donations to active campaigns
- Auto-renew enrollment / opt-out

#### Admin Features:
- Member management (approve, activate, expire)
- Event creation and management
- Announcement publishing
- Payment tracking
- Settings configuration
- Audit log viewing

#### Deployment:
- Served directly by Coterie backend
- Server-side rendered with HTMX for interactivity
- Minimal JavaScript (Alpine.js for UI enhancements)

## Data Flow Examples

### Public Event Display
```
Static Site → GET /public/events → Coterie API → JSON Response → Render on Page
```

### Member Signup
```
Static Site Form → POST /public/signup → Coterie API → Create Pending Member → Email Notification
```

### Admin Member Approval
```
Admin Portal → POST /portal/admin/members/:id/activate → Portal Handler
   → MemberRepository::update + welcome email + audit log + invalidate sessions
   → Integration Manager dispatch (Discord role re-sync, etc.)
```

The portal handler does the full side-effect chain in one place. There
is no JSON `/api/...` equivalent — admin actions are exclusively
served by HTML+HTMX handlers under `/portal/admin/*`.

## Integration Points

### For Static Sites
The public API is designed to be consumed by any static site generator:

```javascript
// Example: Fetching events for a static site
fetch('https://api.yourorg.com/public/events')
  .then(res => res.json())
  .then(events => {
    // Render events on your static site
  });
```

```html
<!-- Example: Embedding signup form -->
<form action="https://api.yourorg.com/public/signup" method="POST">
  <input name="email" type="email" required>
  <input name="username" required>
  <input name="full_name" required>
  <input name="password" type="password" required>
  <button type="submit">Join Us</button>
</form>
```

### For Calendar Apps
```
# Subscribe to events in any calendar app
https://api.yourorg.com/public/feed/calendar
```

### For RSS Readers
```
# Subscribe to announcements
https://api.yourorg.com/public/feed/rss
```

## Security Considerations

### Public API
- Rate limiting on all endpoints
- CORS configured for allowed domains
- No sensitive data exposed

### Management Portal
- Session-based authentication
- Secure cookies (HttpOnly, SameSite=Lax, Secure)
- Role-based access control (member vs admin), declared per-router
- Rate limiting on login (5 **failed** attempts / 15min / account, plus 20
  distinct accounts / 15min / address for sustained abuse — successes cost
  nothing) and money-moving endpoints (10/min/IP)

### CSRF — top-level, secure-by-default

CSRF protection is enforced at the **top of the router** by
`csrf_protect_unless_exempt`. Any state-changing method
(POST/PUT/DELETE/PATCH) on any path is rejected unless it carries a
valid `X-CSRF-Token` header (or `csrf_token` form field) bound to the
caller's session. Adding a new route inherits protection
automatically — there is no way to "forget" CSRF.

The exempt list is small, explicit, and lives in
`src/api/middleware/security.rs`:

- `POST /api/payments/webhook/stripe` — Stripe HMAC signature.
- `POST /public/signup`, `POST /public/donate` — cross-origin POSTs
  from the marketing site, CORS-gated.
- `POST /auth/login` — no session yet to bind a token to.

Adding to that list requires a clear answer to "why can't this carry
a CSRF token?"

### Static Site
- No secrets or API keys needed
- All API calls are to public endpoints
- Can use environment variables for API base URL

## Resilience

Coterie favors a few simple, durable patterns over heavier infrastructure.
The goal is that a downed external API, a slow network call, or a process
restart never loses work or leaves the system inconsistent.

- **Lean on the payment processor's retries.** Inbound Stripe webhooks are the
  source of truth for completed payments. Each event is claimed once in a
  `processed_events` table (exactly-once); if a handler errors, the claim is
  released and a non-2xx returned so Stripe re-delivers — Stripe retries for
  ~3 days. Coterie needs no inbound queue of its own.

- **Reconcile, don't replay.** External state that can drift (e.g. Discord
  roles) is corrected by a daily sweep that recomputes the desired state from
  the database and re-applies it idempotently. Real-time integration calls are
  best-effort: a failure is logged but never fails the admin action that
  triggered it, because the next reconcile converges anyway.

- **Durable, idempotent background work.** Scheduled charges live in a
  `scheduled_payments` table with status and retry-count columns; the billing
  loop just re-scans for due rows each tick, so a restart resumes cleanly.
  Charges carry idempotency keys so a double-run can't double-bill, dues
  extension is a single atomic claim, and reminders/publishes guard on a
  "sent"/"published" timestamp so repeated ticks are no-ops. Terminal failures
  (e.g. a card declined past the retry limit) raise an operator alert delivered
  over Discord *and* email, so one channel being down doesn't bury it.

- **Keep transactions short and SQL-only.** Network calls (Stripe, Discord,
  SMTP) never run inside a database transaction, so a slow or hung external
  call can't hold a write lock. Combined with WAL mode and a `busy_timeout`,
  concurrent admin browsing and background jobs coexist without `SQLITE_BUSY`.

## Deployment Architecture

```
┌─────────────────┐      ┌──────────────────┐      ┌─────────────────┐
│   Static Site   │      │  Coterie Server  │      │    Database     │
│   (CDN/Edge)    │◄────►│    (VPS/Cloud)   │◄────►│    (SQLite)     │
└─────────────────┘      └──────────────────┘      └─────────────────┘
                                 │
                                 ▼
                         ┌──────────────────┐
                         │   Integrations   │
                         │ (Discord, Unifi) │
                         └──────────────────┘
```

## Configuration

### Static Site
Needs only the Coterie API base URL:
```javascript
const COTERIE_API = process.env.COTERIE_API_URL || 'https://api.yourorg.com';
```

### Coterie Server
Configured via environment variables and database settings:
- API keys (Stripe, Discord, etc.) - environment only
- Business logic (fees, text) - database settings via admin UI
- Server config (port, host) - environment variables

## Benefits of This Architecture

1. **Separation of Concerns**: Public content vs management functionality
2. **Independent Deployment**: Static site can be updated without touching the API
3. **Performance**: Static site can be CDN-cached globally
4. **Security**: Management portal behind authentication, public site has no secrets
5. **Flexibility**: Organizations can use any static site generator they prefer
6. **Cost-Effective**: Static hosting is free/cheap, only API server needs compute
7. **Developer Experience**: Frontend developers can work independently on the static site