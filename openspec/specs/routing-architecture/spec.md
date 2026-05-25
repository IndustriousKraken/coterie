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

### Requirement: Setup-redirect check is process-cached after first positive observation

`AppState` SHALL hold an `admin_exists_observed: Arc<AtomicBool>` flag, initialized to `false`. The `require_setup` middleware SHALL consult this flag before querying the database; once set to `true`, the middleware SHALL forward the request without querying. The middleware SHALL set the flag to `true` the first time it observes any admin row in the database.

The flag SHALL be process-local; a multi-process deployment SHALL have each process independently arm its own cache. The flag SHALL be sticky for the lifetime of the process — it SHALL NOT be cleared by any application-level operation. Operators who manually remove admin status from every member via direct SQL SHALL restart the server to re-arm the setup-redirect path.

The setup-wizard handler (POST `/setup`) SHALL set the flag to `true` immediately after successfully creating the first admin, so the very next request bypasses the redundant DB query.

#### Scenario: First request after setup observes admin and arms cache

- **WHEN** an instance has just completed first-boot setup and the very next request arrives at the middleware
- **THEN** the middleware SHALL forward the request, AND `admin_exists_observed` SHALL be `true` afterward (set proactively by the setup-wizard handler or by the middleware itself on a positive DB lookup)

#### Scenario: Subsequent requests skip the DB query

- **WHEN** `admin_exists_observed` is `true` and a non-static, non-setup-prefix request arrives
- **THEN** the middleware SHALL forward the request without running `SELECT 1 FROM members WHERE is_admin = 1 LIMIT 1`

#### Scenario: First-boot redirect still fires before any admin exists

- **WHEN** a fresh-install instance with no admin yet receives a request to a non-static, non-setup-prefix path
- **THEN** the middleware SHALL run the DB query, observe no admin, and respond with a redirect to `/setup`; `admin_exists_observed` SHALL remain `false`

#### Scenario: Concurrent first-boot requests converge

- **WHEN** two concurrent requests reach the middleware before any admin exists, while a third request is concurrently completing the setup-wizard handler
- **THEN** each first-boot request SHALL independently consult the DB, the wizard's admin creation SHALL be serialized by `setup_lock`, and after the wizard completes the next request SHALL observe `admin_exists_observed = true` (set either by the wizard's proactive store or by the middleware's own observation)

#### Scenario: Direct-SQL admin removal does not re-trigger redirect without restart

- **WHEN** an operator directly clears `is_admin` on every member row via DB tooling outside the application
- **THEN** the cached `admin_exists_observed = true` SHALL persist; subsequent requests SHALL continue to forward (not redirect to `/setup`); recovery SHALL require a server restart so the flag re-initializes to `false`

### Requirement: AppState exposes FromRef impls for granular extraction

`AppState` SHALL expose `axum::extract::FromRef<AppState>` implementations for every constituent service, repository, and piece of infrastructure that a handler might reasonably extract. Adding a new field to `AppState` or `ServiceContext` SHALL also include a `FromRef<AppState>` impl in the same change.

The impls SHALL all live in `src/api/state.rs` (or a clearly-scoped sub-module of it) so that "what `AppState` exposes" can be answered by reading a single file.

Handlers MAY continue to use `State<AppState>` extraction; this requirement does not mandate a particular handler style. It only requires that the FromRef machinery is available so granular extraction is possible.

#### Scenario: Every field is extractable

- **WHEN** a handler writes `State(svc): State<Arc<dyn SomeRepository>>` against a router holding `AppState`
- **THEN** the extraction SHALL resolve via `FromRef<AppState>` to `state.service_context.<field>.clone()` (or the analogous path for non-`service_context` fields)

#### Scenario: A new field on AppState gets a FromRef impl

- **WHEN** a contributor adds a new service, repo, or infrastructure component to `AppState` (or to `ServiceContext` reachable through `AppState`)
- **THEN** the same change SHALL include a `FromRef<AppState>` impl for it in `src/api/state.rs`

#### Scenario: Existing State<AppState> handlers still compile

- **WHEN** a handler authored before this change uses `State(state): State<AppState>`
- **THEN** that handler SHALL continue to compile and run unchanged — the FromRef impls coexist with the old extraction shape

### Requirement: Distinct RateLimiter instances are extractable via newtypes

The two `RateLimiter` instances on `AppState` (`login_limiter` and `money_limiter`) SHALL each be wrapped in a newtype (`LoginLimiter`, `MoneyLimiter`) so they can be disambiguated as `State<LoginLimiter>` vs. `State<MoneyLimiter>` extractors. A bare `FromRef<AppState> for RateLimiter` SHALL NOT exist (it would be ambiguous between the two instances).

#### Scenario: Login limiter extracts via its newtype

- **WHEN** a handler writes `State(limiter): State<LoginLimiter>`
- **THEN** the extraction SHALL resolve to a clone of `state.login_limiter` wrapped in the `LoginLimiter` newtype

#### Scenario: Money limiter extracts via its newtype

- **WHEN** a handler writes `State(limiter): State<MoneyLimiter>`
- **THEN** the extraction SHALL resolve to a clone of `state.money_limiter` wrapped in the `MoneyLimiter` newtype

### Requirement: Portal handlers extract granular state, not AppState

Every handler in `src/web/templates/`, `src/web/portal/`, and `src/web/portal/admin/` SHALL extract its dependencies via `State<Arc<dyn TargetService>>` (or analogous granular wrappers) rather than `State<AppState>`. The handler signature SHALL list exactly the services, repositories, and infrastructure components the body actually uses.

Exceptions are allowed for handlers that genuinely use a broad cross-section (≥6 components, or otherwise unreasonable to enumerate). Such exceptions SHALL carry a brief comment explaining the choice. The default position is granular extraction.

Middleware (functions wired via `from_fn_with_state`) SHALL continue to take `State<AppState>` — `FromRef` is for handler extraction, not for middleware.

#### Scenario: New portal handler uses granular extraction

- **WHEN** a contributor adds a new portal handler that uses a few specific services
- **THEN** the handler SHALL extract those services individually via `State<Arc<…>>`; it SHALL NOT default to `State<AppState>` for convenience

#### Scenario: Reader can see a handler's dependencies from its signature

- **WHEN** a reader inspects a portal handler's signature
- **THEN** the granular extractors SHALL enumerate exactly the dependencies the body uses; no domain navigation is needed to discover the actual surface

#### Scenario: Exception is documented at the site

- **WHEN** a contributor retains `State<AppState>` for a handler with broad cross-cutting needs
- **THEN** a brief inline comment SHALL explain why (e.g., "Builds three integration events and reaches five services; granular extraction would yield 7 extractors")

### Requirement: BaseContext takes granular inputs, not AppState

The `BaseContext::for_member` helper SHALL take granular inputs (`csrf_service: &CsrfService`, `current_user: &CurrentUser`, `session: &SessionInfo`) rather than `&AppState`. This is so handlers that build a `BaseContext` for an Askama template can themselves use granular extraction without retaining `State<AppState>` solely to feed the helper.

#### Scenario: Handler building BaseContext uses granular extraction

- **WHEN** a portal handler renders an Askama page using `BaseContext::for_member(...)`
- **THEN** the handler SHALL extract `State<Arc<CsrfService>>` granularly and pass `&csrf_service` to the helper; the handler SHALL NOT retain `State<AppState>` solely for the helper's sake

### Requirement: API handlers extract granular state, not AppState

Every handler in `src/api/handlers/` SHALL extract its dependencies via `State<Arc<dyn TargetService>>` (or analogous granular wrappers, including the `LoginLimiter` / `MoneyLimiter` newtypes for rate limiters) rather than `State<AppState>`. The handler signature SHALL list exactly the services, repositories, and infrastructure components the body uses.

Exceptions are allowed for handlers with broad cross-cutting needs (≥6 extractors or otherwise unwieldy). Such exceptions SHALL carry a brief inline comment explaining why. The default position is granular extraction.

Middleware in `src/api/middleware/` SHALL continue to use `State<AppState>` — `FromRef` is for handler extraction, not for middleware wired via `from_fn_with_state`.

#### Scenario: API handler signature names its dependencies

- **WHEN** a reader inspects any handler in `src/api/handlers/`
- **THEN** the granular extractors SHALL enumerate exactly the dependencies the body uses; no domain navigation needed

#### Scenario: Webhook handler exception is documented

- **WHEN** `stripe_webhook` (or any other handler with genuine cross-cutting needs) retains `State<AppState>`
- **THEN** a brief inline comment SHALL explain the choice; bare `State<AppState>` without justification SHALL be treated as a defect

#### Scenario: Login limiter consumed via its newtype

- **WHEN** the `login` handler extracts the rate limiter
- **THEN** it SHALL extract `State<LoginLimiter>` (the newtype introduced by `add-fromref-impls-on-appstate`), not `State<RateLimiter>` (which doesn't exist as an unambiguous extractor)

### Requirement: GET /setup redirects when an admin already exists

The `/setup` GET handler SHALL check whether an admin already exists (via `check_admin_exists` or the `admin_exists_observed` `AppState` flag) and, if true, redirect to `/login` instead of rendering the setup form.

This complements the existing POST /setup behavior (which already refuses post-bootstrap inside the `setup_lock` guard). After this change, /setup is fully a dead-end once bootstrap is complete: GET redirects away, POST refuses. The security exemption that lets /setup operate without a session is bounded strictly to the bootstrap window.

#### Scenario: GET /setup after admin exists redirects to /login

- **WHEN** a request hits `GET /setup` on an instance where at least one admin already exists
- **THEN** the response SHALL be a 303 (or 302) redirect to `/login`; the setup form HTML SHALL NOT be rendered

#### Scenario: GET /setup before admin exists renders the form

- **WHEN** a request hits `GET /setup` on a fresh instance with no admin
- **THEN** the response SHALL render the setup form HTML (preserving current first-boot behavior)

#### Scenario: Reaches admin_exists_observed cache when populated

- **WHEN** the GET /setup handler runs and `admin_exists_observed` is already `true` (set either by an earlier middleware check or by the wizard's create_admin call)
- **THEN** the handler SHALL consult that cached value rather than re-querying the database, matching the optimization the `cache-has-admin-flag` change introduced for the middleware

