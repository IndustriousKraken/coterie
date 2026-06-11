## ADDED Requirements

### Requirement: Baseline security headers on every response

The `security_headers` middleware SHALL be layered on the application router and SHALL set the following headers on every response (HTML and otherwise):

- `X-Frame-Options: DENY`
- `X-Content-Type-Options: nosniff`
- `Referrer-Policy: strict-origin-when-cross-origin`
- `Content-Security-Policy: default-src 'self'; script-src 'self' 'nonce-<nonce>' 'strict-dynamic' https://js.stripe.com https://unpkg.com; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' https://api.stripe.com; frame-src https://js.stripe.com; frame-ancestors 'none'; object-src 'none'; base-uri 'self'`

#### Scenario: Headers are set on every response

- **WHEN** any HTTP response leaves the application
- **THEN** the four baseline headers above SHALL be present, including on JSON, redirects, error responses, and static assets

### Requirement: CSP uses per-request script nonce, not 'unsafe-inline'

The Content-Security-Policy SHALL omit `'unsafe-inline'` from `script-src` and instead use a per-request nonce (24 random bytes, base64-encoded) plus `'strict-dynamic'`. Each request SHALL get a fresh nonce. The middleware SHALL substitute the placeholder `__CSP_NONCE__` in HTML response bodies with the per-request nonce; templates SHALL stamp this placeholder on every inline `<script>` they emit.

#### Scenario: Inline script without matching nonce does not execute

- **WHEN** an attacker injects `<script>alert(1)</script>` into an HTML page
- **THEN** the browser SHALL refuse to execute the injected script because it carries no matching nonce

#### Scenario: Two requests get distinct nonces

- **WHEN** the same template is rendered twice for two different requests
- **THEN** the two responses SHALL carry distinct nonces in the CSP header AND in the rewritten `<script nonce="...">` attributes

#### Scenario: Non-HTML responses skip the body rewrite

- **WHEN** a response with `Content-Type` that does not start with `text/html` passes through the middleware
- **THEN** the body SHALL pass through untouched; only the headers SHALL be added

#### Scenario: HTML body without placeholder is not modified

- **WHEN** an HTML response (e.g., partial HTMX fragment with no scripts) does not contain `__CSP_NONCE__`
- **THEN** the body SHALL pass through untouched without being decoded/re-encoded

### Requirement: HSTS only when cookies are secure

The middleware SHALL emit `Strict-Transport-Security: max-age=31536000; includeSubDomains` ONLY when the configured cookies-are-secure flag is true. On dev/HTTP deployments, HSTS SHALL be omitted (browsers ignore HSTS over plain HTTP, but sending it would be noise).

#### Scenario: Dev deployment skips HSTS

- **WHEN** the application runs with `cookies_are_secure() = false`
- **THEN** responses SHALL NOT include the `Strict-Transport-Security` header

### Requirement: HTML rewrite is bounded in memory

The body-rewrite path SHALL cap the buffered body at 4 MB. Bodies larger than the cap SHALL be replaced with an empty body and the failure logged; mangled HTML SHALL never be returned.

#### Scenario: Oversized response is replaced with empty body, not corrupted

- **WHEN** a response body exceeds 4 MB during nonce-rewrite
- **THEN** the middleware SHALL log the error and return an empty body rather than partial/corrupted HTML

### Requirement: Session cookies use secure defaults

Authentication-related cookies (`session`) SHALL be set with `HttpOnly`, `Secure` (when `cookies_are_secure() = true`), and `SameSite=Lax` attributes. JavaScript SHALL NOT be able to read the session cookie via `document.cookie`.

#### Scenario: Session cookie carries HttpOnly + Secure + SameSite=Lax

- **WHEN** a successful login sets the session cookie on a TLS deployment
- **THEN** the `Set-Cookie` header SHALL include `HttpOnly`, `Secure`, and `SameSite=Lax`

#### Scenario: SameSite=Lax mitigates login CSRF

- **WHEN** an attacker hosts a cross-site form auto-submitting a login POST
- **THEN** the browser SHALL NOT include the victim's session cookie due to `SameSite=Lax`, mitigating login-CSRF in concert with the explicit Origin/Referer checks on `/auth/login`
