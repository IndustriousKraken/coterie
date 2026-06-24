## 1. Add the web-auth POST paths to the CSRF exempt list

- [x] 1.1 In `src/api/middleware/security.rs`, extend the `CSRF_EXEMPT_PATHS`
  array (currently at lines 82-88) to add these five entries, keeping the
  existing entries intact:
  - `("POST", "/login")`
  - `("POST", "/login/totp")`
  - `("POST", "/forgot-password")`
  - `("POST", "/reset-password")`
  - `("POST", "/setup")`
- [x] 1.2 Extend the doc comment block above `CSRF_EXEMPT_PATHS` so each new
  entry carries its "cannot carry a session-bound CSRF token because…"
  justification:
  - `/login` — the browser login form (`login.html`) posts here; no `session`
    cookie exists yet to bind a token to.
  - `/login/totp` — the second-factor step; the caller holds only a
    `pending_login` cookie, not a `session` cookie.
  - `/forgot-password` — anonymous reset request; no session; gated by the
    per-IP login rate limiter and an enumeration-safe response.
  - `/reset-password` — anonymous; authorization is the single-use,
    time-limited reset token in the form body, not a session.
  - `/setup` — first-run admin-creation wizard; runs before any admin or
    session exists; gated by the one-shot "no admin yet" check + `setup_lock`.
- [x] 1.3 Do NOT remove the existing `("POST", "/auth/login")` and
  `("POST", "/auth/login/totp")` entries — the JSON login handlers they refer to
  still exist in `src/api/mod.rs` and are still legitimately session-less.

## 2. Regression test: anonymous web-auth POSTs pass the CSRF layer

- [x] 2.1 Add an integration test (e.g. `tests/csrf_web_auth_exempt_test.rs`)
  that builds the FULL merged application exactly as `main.rs` does — mirror the
  `build_app()` helper in `tests/csrf_coverage_test.rs:45-157`: construct the
  `AppState`, then
  `api::create_app(state).merge(web::create_web_routes(state)).layer(require_setup).layer(csrf_protect_unless_exempt)`.
- [x] 2.2 For each path in `["/login", "/login/totp", "/forgot-password",
  "/reset-password", "/setup"]`, send a POST with NO `session` cookie (use a
  well-formed body for its content type — e.g. JSON `{"username":"nobody",
  "password":"x"}` for `/login`) and assert the response status is NOT
  `StatusCode::FORBIDDEN`. Add a comment: a 403 here is the CSRF-layer rejection
  this change fixes; any other status proves the request reached its handler.
- [x] 2.3 Keep one assertion that a still-protected route remains guarded — POST
  `/portal/admin/members/<uuid>/update` with no session SHALL still return
  `403 Forbidden` — so the test proves the exempt list was widened precisely, not
  that CSRF was globally disabled.

## 3. Spec delta

- [x] 3.1 The `specs/csrf-protection/spec.md` delta in this change updates the
  "Exempt list is small, explicit, and justified" requirement (enumeration +
  new scenario). No code action beyond Task 1 is needed; canon is folded at
  archive time.
- [x] 3.2 Run `openspec validate --strict` for this change and resolve any
  reported issues before marking the change ready.
