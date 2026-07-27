# csrf-protection Specification

## MODIFIED Requirements

### Requirement: Exempt list is small, explicit, and justified

The set of CSRF-exempt paths SHALL be a static list in `src/api/middleware/security.rs` named `CSRF_EXEMPT_PATHS`. Adding to the list SHALL require a documented "this endpoint cannot carry a session-bound CSRF token because…" justification.

The exempt entries SHALL include every state-changing endpoint whose caller has no `session` cookie at the time of the request — otherwise the top-level CSRF layer rejects it with 403 before its handler runs. The current exempt entries are:

- `POST /api/payments/webhook/stripe` — Stripe HMAC signature is the auth.
- `POST /public/signup` — cross-origin from marketing site; gated by CORS allowlist + rate limit + bot challenge.
- `POST /public/donate` — same as signup.
- `POST /public/events/:id/register` — public paid-event registration; the caller is an anonymous visitor with no session, so there is no session id to bind a token to. Gated by CORS allowlist + `money_limiter` + bot challenge, in that order, and further constrained by serving only `Public`-visibility events that carry a guest price.
- `POST /login` — the browser portal login form (`templates/auth/login.html`) posts here; no session exists yet to bind a token to. Gated by the per-IP login rate limiter and SameSite=Lax cookies.
- `POST /login/totp` — the second-factor step of the portal login; the caller holds only a `pending_login` cookie at this step, not a `session` cookie, so there is no session id to bind a CSRF token to.
- `POST /forgot-password` — anonymous password-reset request; no session. Gated by the per-IP login rate limiter and an enumeration-safe response.
- `POST /reset-password` — anonymous; authorization is the single-use, time-limited reset token carried in the form body, not a session.
- `POST /setup` — the first-run admin-creation wizard; runs before any admin or session exists. Gated by the one-shot "no admin yet" check + `setup_lock`.
- `POST /auth/login` — JSON login API; no session exists yet to bind a token to.
- `POST /auth/login/totp` — JSON second-factor API; same reason as `/auth/login`: the caller holds only a `pending_login` cookie, not a `session` cookie.

`POST /logout` and `POST /auth/logout` are NOT exempt: every authenticated page renders a CSRF meta tag, the caller holds a valid session, and forced logout warrants protection.

#### Scenario: Adding to the exempt list requires a justification

- **WHEN** a change adds an entry to `CSRF_EXEMPT_PATHS`
- **THEN** the change description MUST state why the endpoint cannot carry a session-bound token

#### Scenario: Anonymous web-auth POST is exempt, not rejected

- **WHEN** an anonymous POST to `/login`, `/login/totp`, `/forgot-password`, `/reset-password`, or `/setup` arrives with no `session` cookie
- **THEN** the top-level CSRF middleware SHALL treat the path as exempt and forward the request to its handler, NOT respond 403 — because these endpoints exist to authenticate (or first-provision) a caller who has no session yet and therefore cannot present a session-bound CSRF token

#### Scenario: Logout is not exempt

- **WHEN** a logout POST arrives without a valid CSRF token
- **THEN** the middleware SHALL reject it with 403 Forbidden

#### Scenario: Anonymous public event registration is exempt, not rejected

- **WHEN** an anonymous `POST /public/events/:id/register` arrives with no `session` cookie
- **THEN** the CSRF middleware SHALL treat the path as exempt and forward it to its handler, which applies the rate limit and bot challenge in place of a session-bound token
