# csrf-protection Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: CSRF enforcement is top-level on the merged router

CSRF validation SHALL be layered at the outermost level of the merged application router (after `Router::merge` of API and portal routers). Per-route or per-router CSRF middleware MUST NOT be relied on; the contract lives at the top of the call graph and only there.

#### Scenario: A new state-changing route inherits CSRF protection

- **WHEN** a contributor adds a new POST/PUT/DELETE/PATCH route under `/portal/*` or `/api/*` without explicitly layering any CSRF middleware
- **THEN** the route SHALL still require a valid CSRF token because the top-level layer runs first

#### Scenario: CSRF middleware is not layered before a merge

- **WHEN** the application router is constructed
- **THEN** CSRF middleware SHALL NOT be applied inside `create_app` or `create_portal_routes` (axum 0.7 layers do not propagate through `Router::merge`); it SHALL be applied once on the merged router in `main.rs`

### Requirement: State-changing requests require a session-bound token

For non-exempt POST/PUT/DELETE/PATCH requests, the system SHALL require both a valid session cookie AND a CSRF token bound to that session. The token SHALL be supplied via:

- the `X-CSRF-Token` header (HTMX, fetch), OR
- a `csrf_token` form field in `application/x-www-form-urlencoded` bodies, OR
- a `csrf_token` field in `multipart/form-data` bodies (for image-upload forms).

JSON request bodies that carry no header SHALL be rejected.

#### Scenario: HTMX request with valid token is accepted

- **WHEN** an authenticated HTMX request issues a POST with `X-CSRF-Token` header bound to its session
- **THEN** the middleware SHALL validate the token and forward the request to the handler

#### Scenario: Form POST with valid csrf_token field is accepted

- **WHEN** an authenticated POST is sent with `Content-Type: application/x-www-form-urlencoded` and a `csrf_token` form field bound to the session
- **THEN** the middleware SHALL validate the token, restore the buffered body, and forward the request

#### Scenario: Multipart POST with valid csrf_token field is accepted

- **WHEN** an authenticated POST is sent with `Content-Type: multipart/form-data` and `csrf_token` as the first field bound to the session
- **THEN** the middleware SHALL validate the token, buffer the body up to 12MB, and forward the request

#### Scenario: Missing session is rejected

- **WHEN** a non-exempt state-changing request arrives without a session cookie
- **THEN** the middleware SHALL respond with 403 Forbidden

#### Scenario: Invalid token is rejected

- **WHEN** a non-exempt state-changing request arrives with a session cookie but an invalid or missing CSRF token
- **THEN** the middleware SHALL respond with 403 Forbidden

#### Scenario: JSON body without header is rejected

- **WHEN** a state-changing JSON request is sent with `Content-Type: application/json` and no `X-CSRF-Token` header
- **THEN** the middleware SHALL respond with 403 Forbidden

### Requirement: Read-only methods pass through

The middleware SHALL pass GET, HEAD, and OPTIONS requests through without CSRF validation.

#### Scenario: GET request never requires a token

- **WHEN** a GET request reaches the CSRF layer
- **THEN** the request SHALL be forwarded without token validation

### Requirement: Exempt list is small, explicit, and justified

The set of CSRF-exempt paths SHALL be a static list in `src/api/middleware/security.rs` named `CSRF_EXEMPT_PATHS`. Adding to the list SHALL require a documented "this endpoint cannot carry a session-bound CSRF token because…" justification.

The current exempt entries are:
- `POST /api/payments/webhook/stripe` — Stripe HMAC signature is the auth.
- `POST /public/signup` — cross-origin from marketing site; gated by CORS allowlist + rate limit + bot challenge.
- `POST /public/donate` — same as signup.
- `POST /auth/login` — no session exists yet to bind a token to.

`POST /auth/logout` is NOT exempt: every authenticated page renders a CSRF meta tag, and forced logout warrants protection.

#### Scenario: Adding to the exempt list requires a justification

- **WHEN** a change adds an entry to `CSRF_EXEMPT_PATHS`
- **THEN** the change description MUST state why the endpoint cannot carry a session-bound token

#### Scenario: Logout is not exempt

- **WHEN** a logout POST arrives without a valid CSRF token
- **THEN** the middleware SHALL reject it with 403 Forbidden

### Requirement: SessionInfo is injected on success

When CSRF validation passes, the middleware SHALL insert a `SessionInfo` value into the request extensions so downstream auth middleware does not need to re-read the session cookie.

#### Scenario: Downstream middleware sees SessionInfo

- **WHEN** the CSRF layer validates a request successfully
- **THEN** `request.extensions().get::<SessionInfo>()` SHALL return the session id for downstream consumers

