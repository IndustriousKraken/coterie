## Why

Coterie has grown into a production system with three HTTP surfaces, layered security defaults, and a service/repository architecture — but none of it is captured as enforceable specs. The CLAUDE.md prose is load-bearing (a CSRF gap was reintroduced once and only caught because a contributor knew the rule), and onboarding any future contributor or AI agent currently depends on reading the code top-to-bottom. Capturing the existing behavior as OpenSpec specs gives us a contract future changes are checked against, makes the secure-by-default rules machine-verifiable, and turns "what does this app do?" from a code-reading exercise into a spec read.

## What Changes

- Reverse-engineer every existing user-visible and infrastructural capability into OpenSpec specs (specs only — **no behavior change**).
- Group requirements by capability with kebab-case names; one `specs/<name>/spec.md` per capability.
- Encode the three-surface routing contract (`/portal`, `/public`, `/api`) as a top-level capability so future routes are checkable against it.
- Encode the secure-by-default rules (top-level CSRF, per-router auth gating, rate limits on money-moving paths, security headers, cookie defaults) as their own capabilities — each one a rule someone could violate.
- Encode service-layer side-effect rules (audit log + integration events live in services, not handlers) so the spec catches handlers that skip them.
- This is documentation-only: no production source files change.

## Capabilities

### New Capabilities

**Routing & cross-cutting security**
- `routing-architecture`: The three-surface contract — `/portal/*` HTML+HTMX (only admin surface), `/public/*` JSON for marketing site, `/api/*` narrow JSON (Stripe webhook + saved-card). Forbids admin shape on `/api/*` and `/admin/*`.
- `csrf-protection`: Top-level enforcement on the merged router; explicit `CSRF_EXEMPT_PATHS` list; rationale required for each exempt path.
- `auth-middleware-tiers`: `require_admin_redirect`, `require_auth_redirect`, `require_restorable`, `require_auth` — every router declares one or is explicitly public.
- `rate-limiting`: Per-IP limits on `/auth/login` and money-moving endpoints; a money-moving endpoint without a rate limit is a defect.
- `security-headers`: Global response headers; HttpOnly / Secure / SameSite=Lax cookie defaults.
- `cors-policy`: Same-origin by default; allowed origins from `cors_origins` setting; the cross-origin gate for `/public/*` POSTs.
- `bot-challenge`: Bot-gating challenge fronting public-API state-changing endpoints (signup, donate).

**Authentication & sessions**
- `session-auth`: Login, logout, session lifecycle, Origin/Referer checks on `/auth/login`.
- `password-management`: Password set, change, reset flow.
- `email-tokens`: Single-use email tokens for verification / restoration links.
- `totp-2fa`: TOTP enrollment, challenge, secret encryption at rest.
- `recovery-codes`: Generation, single-use redemption, regeneration.

**Public API (`/public/*`)**
- `public-signup`: New-member signup endpoint; CORS-gated, bot-challenged, rate-limited.
- `public-donate`: One-off donation endpoint; same gates as signup; routes through Stripe.
- `public-content-feeds`: Public reads of events and announcements, RSS feed, iCal feed.

**Admin portal (`/portal/admin/*`)**
- `admin-members`: Member CRUD, status transitions, admin flag, dues / billing fields, bulk operations.
- `admin-events`: Event CRUD plus recurring-event series management.
- `admin-announcements`: Announcement CRUD, publish state.
- `admin-billing-dashboard`: Billing overview, scheduled payments view, dunning state.
- `admin-payments`: Manual payment recording, refunds, payment history.
- `admin-settings`: Org settings, secret rotation surfaces.
- `admin-types`: Membership types, event types, announcement types (configurable enums).
- `admin-audit-log`: Audit-log viewer, filtering, export.
- `admin-integrations`: Discord configuration, email-template configuration / send-test surfaces.

**Member portal (`/portal/*`, member-tier)**
- `member-dashboard`: Landing page, dues status, upcoming events.
- `member-profile`: Profile edit, password change, 2FA management.
- `member-content`: Member-visible events and announcements lists.
- `member-saved-cards`: List, add (Stripe.js + SetupIntent), remove saved cards.
- `member-donations`: Member-initiated donation flow.
- `dues-restoration`: Restoration flow for Expired members (`require_restorable`-gated).

**Billing & payments**
- `stripe-webhook`: HMAC-signature-authed webhook at `/api/payments/webhook/stripe`; idempotent event handling via processed-events table; dispatcher seam for tests.
- `saved-card-management`: JSON `/api/*` endpoints called by Stripe.js for SetupIntent flow; member-self-auth.
- `recurring-billing`: Background billing runner — schedule, retries, dunning, cancellation rules.
- `scheduled-payments`: Domain model and lifecycle for upcoming/scheduled payments.
- `payment-recording`: Manual (admin) and automated (webhook) payment recording paths share a service so audit + integration-event parity is guaranteed.

**External integrations**
- `discord-integration`: Outbound Discord notifications, configuration model.
- `unifi-integration`: UniFi access integration (network gating).
- `admin-alert-email`: Outbound admin-alert email channel for security / billing events.

**Domain & data layer**
- `domain-types`: Sum types over nullable columns (Payer, PaymentKind, StripeRef, etc.); validation at boundary; trust internal callers.
- `repository-contracts`: Repository trait in `src/repository/mod.rs`; concurrency / idempotency / conflict semantics documented on the trait, not just in the impl.

**Cross-cutting service rules**
- `audit-logging`: Every state-changing admin action emits an audit-log entry from the service layer.
- `integration-events`: State changes that external systems care about emit integration events from the service layer; handlers cannot skip them.

### Modified Capabilities

None — there are no existing specs in `openspec/specs/`. Every capability above is new.

## Impact

- **Code**: No production source changes. Documentation-only effort.
- **`openspec/specs/`**: ~37 new spec directories created.
- **Future workflow**: New changes can begin to reference / amend specific specs rather than restating the rule. The CLAUDE.md prose can later be trimmed once the specs cover its load-bearing parts (out of scope for this change — CLAUDE.md stays as-is).
- **Effort**: Substantial. Each capability needs requirement statements + at least one `#### Scenario:` block. Treat as a multi-session effort; tasks.md will sequence the spec authoring so it can be done in passes.
- **Risk**: Specs that drift from code are worse than no specs. Each capability spec must be checked against the corresponding source files at write time, and any discrepancy resolved by updating the spec to match observed behavior (not by changing code).
