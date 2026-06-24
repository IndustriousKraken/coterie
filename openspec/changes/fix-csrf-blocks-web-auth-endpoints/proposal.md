## Why

The top-level CSRF layer rejects every browser-facing authentication POST,
locking the entire web portal out of its own login, 2FA, password-reset, and
first-run-setup flows.

`csrf_protect_unless_exempt` is layered as the OUTERMOST middleware on the
merged application router in `src/main.rs:541-550` (CSRF is added last, so it
runs first). For any state-changing method that is NOT GET/HEAD/OPTIONS and NOT
in `CSRF_EXEMPT_PATHS`, the middleware requires a `session` cookie before it will
look at anything else:

```rust
// src/api/middleware/security.rs:132
let session_cookie = jar.get("session").ok_or(AppError::Forbidden)?;
```

The exempt list (`src/api/middleware/security.rs:82-88`) names the JSON API
auth endpoints `/auth/login` and `/auth/login/totp` — but the browser portal's
forms POST to the WEB routes, which are different paths and are NOT exempt:

- `templates/auth/login.html:21` → `hx-post="/login"`
- `templates/auth/login_totp.html:18` → `hx-post="/login/totp"`
- `templates/auth/forgot_password.html:24` → `action="/forgot-password"`
- `templates/auth/reset_password.html:16` → `action="/reset-password"`
- `templates/auth/setup.html:18` → `hx-post="/setup"`

These routes are registered in `src/web/mod.rs:31-57` and merged into the app,
so the top-level CSRF layer covers them (the same way it covers `/portal/*` —
see `tests/csrf_coverage_test.rs`, which proves a session-less POST to a merged
web route is rejected with 403). The callers of all five are by definition
session-less: `login_page` (`src/web/templates/auth.rs:51-82`) renders the login
form WITHOUT setting any `session` cookie; the `/login/totp` step holds only a
`pending_login` cookie; `/forgot-password` and `/reset-password` are anonymous;
`/setup` runs before any admin or session exists.

**Concrete harm (availability / authentication lockout).** Every anonymous POST
to `/login`, `/login/totp`, `/forgot-password`, `/reset-password`, or `/setup`
hits the `jar.get("session").ok_or(AppError::Forbidden)` branch and returns
**403 Forbidden before the handler ever runs**. The result:

- Nobody can log in through the portal (the only admin surface — see README).
- A member with 2FA cannot complete the second factor.
- Nobody can request or complete a password reset.
- A freshly provisioned instance cannot run the in-app first-run setup wizard.

A user already holding a live `session` cookie is unaffected, which is why the
break is invisible until a session expires or a new user arrives — and why no
test catches it: the web-auth integration tests
(`tests/web_login_session_create_fail.rs:62-64`, `tests/web_login_totp.rs`)
build `web::create_web_routes(state)` in ISOLATION, bypassing the top-level CSRF
+ setup layers, so they exercise the handler but never the production middleware
stack.

**Contract change (stated plainly).** The canonical `csrf-protection` spec's
"Exempt list is small, explicit, and justified" requirement enumerates the
exempt paths and currently lists `POST /auth/login` / `POST /auth/login/totp`.
This change corrects that enumerated, observable contract: the actual web-auth
POST endpoints (`/login`, `/login/totp`, `/forgot-password`, `/reset-password`,
`/setup`) are added to the exempt set, because — exactly like `/public/signup`
and `/auth/login` already on the list — they exist to authenticate a caller who
has no session yet and so cannot carry a session-bound CSRF token. Their
non-CSRF protections remain: the per-IP login/`money` rate limiters,
SameSite=Lax cookies, the enumeration-safe forgot-password response, the
single-use time-limited reset token, and the one-shot `setup_lock` +
"no admin yet" gate.

## What Changes

- Add the five session-less web-auth POST paths to `CSRF_EXEMPT_PATHS` in
  `src/api/middleware/security.rs`, each with the required justification comment:
  `POST /login`, `POST /login/totp`, `POST /forgot-password`,
  `POST /reset-password`, `POST /setup`. The existing entries (including the JSON
  `/auth/login` and `/auth/login/totp`, which still exist in `src/api/mod.rs`)
  are retained.
- Update the doc comment above the list so each new entry carries its
  "cannot carry a session-bound CSRF token because…" rationale.
- Update the `csrf-protection` capability spec's "Exempt list is small,
  explicit, and justified" requirement to enumerate the web-auth paths and add a
  scenario asserting an anonymous web-auth POST is forwarded (not 403'd).
- Add a regression integration test that builds the FULL merged app (CSRF layer
  included, the way `tests/csrf_coverage_test.rs` does) and asserts each of the
  five anonymous web-auth POSTs is NOT rejected with 403 by the CSRF layer.

## Impact

- `src/api/middleware/security.rs` — extend `CSRF_EXEMPT_PATHS` and its doc
  comment.
- `openspec/specs/csrf-protection/spec.md` — modified by this change's
  `specs/csrf-protection/spec.md` delta (folded into canon at archive time).
- `tests/` — new full-stack regression test proving the web-auth POSTs pass the
  CSRF layer. (The existing isolated web-login tests stay; they do not cover the
  merged middleware stack.)
- No operator action required.
