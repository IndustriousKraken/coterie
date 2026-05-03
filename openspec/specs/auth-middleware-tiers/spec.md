# auth-middleware-tiers Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Every leaf router declares its auth tier

Every `Router::new()` that registers leaf routes (`.route(...)`) in `src/api/mod.rs`, `src/web/portal/mod.rs`, and `src/web/mod.rs` SHALL fall into exactly one of the following categories:

1. Layer one of the auth middleware values via `route_layer`:
   - `require_admin_redirect` — admin-only portal routes
   - `require_auth_redirect` — Active/Honorary member portal routes
   - `require_restorable` — Active, Honorary, OR Expired member portal routes (dues-restoration scope)
   - `require_auth` — authenticated JSON (member-self saved-card APIs)

2. Be a deliberately-public router whose purpose is documented at the function or surrounding comment:
   - `public_routes` in `src/api/mod.rs` — `/public/*` cross-origin endpoints; bot-challenge / CORS / rate-limit gate them.
   - The outer router in `create_app` — registers `/`, `/health`, `/api`, `/api/docs`, `/auth/login`, `/auth/logout`, and nests `/api` + `/public`. Auth/health/docs are intentionally pre-session or public; `/api/*` and `/public/*` carry their own gates inside.
   - The outer router in `create_web_routes` — registers pre-session pages (`/login`, `/setup`, `/verify`, `/forgot-password`, `/reset-password`, etc.), serves `/static/*` and `/uploads/:filename`, and nests `/portal`. Pre-session pages cannot have a session-based gate; `/uploads` performs a per-file auth check inside the handler.

3. Be a pure container `Router::new()` whose only operations are `.merge()` / `.nest()` of already-gated child routers. The container itself adds no leaf routes and is therefore not subject to gating.

There is no acceptable leaf router that silently omits a gate.

#### Scenario: A new portal admin route is gated

- **WHEN** a new admin route is added to the admin router in `src/web/portal/mod.rs`
- **THEN** it inherits `require_admin_redirect` from the router-level `route_layer`; a router that omits the gate is forbidden

#### Scenario: A public route is explicitly marked

- **WHEN** an endpoint is intentionally public (e.g., `/public/events`)
- **THEN** the endpoint SHALL live on the `public_routes` router and the absence of an auth gate SHALL be obvious from the surrounding context

### Requirement: require_admin_redirect enforces Active/Honorary AND admin flag AND optional TOTP

`require_admin_redirect` SHALL allow the request through ONLY if all of the following hold:

1. A valid session cookie maps to an existing member.
2. The member's status is `Active` or `Honorary`.
3. The member has `is_admin = true`.
4. If the `auth.require_totp_for_admins` setting is `true`, the member has TOTP enrolled.

A non-admin authenticated member SHALL be redirected to `/portal/dashboard`. An admin without required TOTP SHALL be redirected to `/portal/profile/security?reason=admin_totp_required`. Anonymous, expired, suspended, or pending users SHALL be redirected to `/login?redirect=<original-uri>`.

#### Scenario: Non-admin member is redirected to dashboard

- **WHEN** an authenticated Active member without `is_admin = true` requests `/portal/admin/members`
- **THEN** the middleware SHALL respond with a redirect to `/portal/dashboard`

#### Scenario: Admin without TOTP is redirected when setting is enabled

- **WHEN** `auth.require_totp_for_admins` is `true` and an admin without TOTP enrollment requests an admin route
- **THEN** the middleware SHALL respond with a redirect to `/portal/profile/security?reason=admin_totp_required`

#### Scenario: Anonymous request is redirected to login with redirect param

- **WHEN** an unauthenticated request reaches an admin route at `/portal/admin/members/123`
- **THEN** the middleware SHALL redirect to `/login?redirect=%2Fportal%2Fadmin%2Fmembers%2F123`

#### Scenario: Setting lookup failure defaults to not enforced

- **WHEN** the lookup of `auth.require_totp_for_admins` fails (e.g., row missing)
- **THEN** the middleware SHALL treat it as not enforced, so a setup hiccup does not lock every admin out

### Requirement: require_auth_redirect routes Expired members to restoration

`require_auth_redirect` SHALL allow Active/Honorary members through, redirect Expired members to `/portal/restore`, and redirect everyone else to `/login?redirect=<original-uri>`.

#### Scenario: Expired member is sent to restoration flow

- **WHEN** an Expired member requests `/portal/dashboard`
- **THEN** the middleware SHALL respond with a redirect to `/portal/restore`

#### Scenario: Active member passes through

- **WHEN** an Active member requests `/portal/dashboard`
- **THEN** the middleware SHALL inject `CurrentUser` and `SessionInfo` and forward the request

### Requirement: require_restorable allows Expired alongside Active/Honorary

`require_restorable` SHALL allow members with status Active, Honorary, OR Expired through. It SHALL be applied to and only to the routes an Expired member needs to view dues, manage cards, pay, and pull historical receipts.

#### Scenario: Expired member can view payment methods

- **WHEN** an Expired member requests `/portal/payments/methods`
- **THEN** the middleware SHALL forward the request

#### Scenario: Expired member cannot reach Active-only routes

- **WHEN** an Expired member requests `/portal/events`
- **THEN** the route is gated by `require_auth_redirect`, which SHALL redirect to `/portal/restore`

### Requirement: require_auth returns JSON-friendly errors for /api/* member routes

`require_auth` SHALL be used on `/api/*` member-self routes (saved-card management) and SHALL return `Unauthorized`/`Forbidden` errors rather than redirects.

#### Scenario: Anonymous request to /api/payments/cards returns 401

- **WHEN** an anonymous request hits `GET /api/payments/cards`
- **THEN** the response SHALL be 401 Unauthorized, not a redirect

#### Scenario: Pending member receives 403

- **WHEN** an authenticated Pending-status member hits an `/api/*` member route
- **THEN** the response SHALL be 403 Forbidden

