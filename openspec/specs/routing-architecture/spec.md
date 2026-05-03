# routing-architecture Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Three primary HTTP surfaces with distinct purposes

The system SHALL expose three primary HTTP surfaces, each with a distinct, narrow purpose:

- `/portal/*` — server-rendered HTML with HTMX. The ONLY admin surface. Session-cookie auth.
- `/public/*` — JSON API consumed cross-origin by the organization's marketing/static site. CORS-gated; no session.
- `/api/*` — narrow JSON. Limited to (1) the Stripe webhook, (2) saved-card management endpoints called directly by Stripe.js from the portal, and (3) GET-only metadata endpoints for the OpenAPI spec and Swagger UI (`/api`, `/api/docs`, `/api/docs/openapi.json`).

Auxiliary routes outside these surfaces are limited to:

- Pre-session auth/onboarding pages: `/login`, `/login/totp`, `/auth/login`, `/logout`, `/auth/logout`, `/setup`, `/verify`, `/forgot-password`, `/reset-password`. These exist outside the three surfaces because no session exists yet to bind them to.
- Static asset serving: `/static/*` (no auth) and `/uploads/:filename` (per-file auth check inside the handler).
- Root/health: `/`, `/health`.

Adding new categories of auxiliary routes outside the three surfaces SHALL require explicit justification.

#### Scenario: Adding an admin endpoint to /api/* is forbidden

- **WHEN** a contributor proposes adding any state-changing admin endpoint (member CRUD, event CRUD, announcement CRUD, payment recording, settings, types, audit) to `/api/*`
- **THEN** the change MUST be rejected; admin actions belong under `/portal/admin/*`

#### Scenario: Adding an admin endpoint to /admin/* is forbidden

- **WHEN** a contributor proposes adding any router mounted at `/admin/*` outside of `/portal/admin/*`
- **THEN** the change MUST be rejected; the JSON `/admin/*` surface was deliberately removed in 2026-04 and must not return

#### Scenario: A new public-marketing endpoint goes under /public/*

- **WHEN** a feature requires a JSON endpoint callable by the marketing site
- **THEN** it SHALL be added under `/public/*` and documented in `src/api/docs.rs` so the OpenAPI spec stays accurate

### Requirement: Single AppState shared across surfaces

The application SHALL construct exactly one `AppState` per process and share it across the API router and the portal/web router. Per-IP rate limiters and the first-boot setup lock MUST be shared between surfaces.

#### Scenario: Login attempts are counted across both surfaces

- **WHEN** the same IP attempts to log in via `/auth/login` and a legacy `/login` path within the rate-limit window
- **THEN** both attempts SHALL count against the same per-IP budget; constructing two `AppState` values would double the effective budget and is forbidden

#### Scenario: First-boot setup is single-flight

- **WHEN** two concurrent requests reach the setup-wizard handler before any admin exists
- **THEN** the shared `setup_lock` SHALL ensure only one succeeds; the other SHALL observe the now-existing admin state

### Requirement: Side effects live in services, not handlers

State-changing admin actions SHALL emit their audit-log entries and integration events from the service layer, not from handlers. The service is the single source of truth for the side-effect set; handlers MUST NOT perform side-effects directly that could be skipped by an alternative caller.

#### Scenario: A new admin action gets its side effects automatically

- **WHEN** a new admin action is added that calls an existing service method
- **THEN** the audit-log entry and integration event for that action SHALL be emitted by the service, not by the handler, so a hypothetical second caller of the same service method gets identical side-effects

