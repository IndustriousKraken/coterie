# Tasks

Every task here is a security control or an operability control. The two rules
that must not be relaxed while implementing: **the response never gets more
specific than it is today**, and **no credential reaches a log**.

## 0. Fix the layer placement first

Everything else in this change is invisible without it.

- [x] 0.1 Move `TraceLayer::new_for_http()` out of `api::create_app` and onto the
  merged router in `src/main.rs`, applied outermost so CSRF- and setup-rejected
  requests are logged too. **(already applied and compile-checked)**
- [x] 0.2 Add a regression test asserting a request to a portal route (e.g. `GET
  /login`) produces a request-scoped log event. Without a test this silently
  regresses the next time someone tidies the router.
- [x] 0.3 Confirm no double-logging: the layer must appear exactly once in the
  final router.

## 0b. Client-IP resolution visibility

- [x] 0b.1 Log the resolved client-IP mode once at startup: forwarded headers
  trusted or not, and whether that was explicit config or scheme inference. Log
  the resolved session-cookie `Secure` flag the same way — same inference source,
  same invisibility.
- [x] 0b.1b Report provenance, not just value: "(inferred from https base URL)"
  vs "(explicitly configured)". Same behavior today, different exposure to a
  future base-URL change.
- [x] 0b.2 Warn when the effective mode would collapse every caller onto the
  localhost placeholder — that turns per-IP rate limiting into one shared bucket
  for the whole organization, and today nothing says so.
- [x] 0b.3 (Record of a prior finding, no code change.) Verified on production
  2026-07-29 that bucketing is genuinely per-IP:
  one IP received `429` while a different IP's request reached the login handler
  in the same second. No fix needed to the resolution logic itself; this section
  is about making the mode observable before it silently changes.

## 1. Shared shape

- [x] 1.1 Define the field set once (a small helper or macro) so every call site
  emits the same `event` / `outcome` / `member_id` / `ip` / `reason` fields. Ad-hoc
  `tracing::warn!("login failed for {}", email)` calls are the thing this change
  exists to replace — structured fields, not interpolated prose.
- [x] 1.2 Add an identifier-redaction helper: if the submitted identifier does not
  parse as an email, log a placeholder instead of the value. Guards against a
  password typed into the email field being persisted in logs.
- [x] 1.3 Resolve the caller IP the same way the rate limiter already does, so the
  two agree when correlating a limit trip with the attempts that caused it.

## 2. Login and second factor

- [x] 2.1 `auth.login` on both surfaces (`/login` web, `/auth/login` JSON):
  `ok` at info; `denied` at warn with `unknown_email` / `bad_password` /
  `inactive_status`.
- [x] 2.2 Verify the responses are byte-identical across those denial reasons —
  add or extend a test asserting it. The log gets the distinction; the caller
  must not.
- [x] 2.3 `auth.totp` for TOTP and recovery-code attempts, including the
  `totp_expired_pending` case where the `pending_login` cookie has aged out.
  Keep the existing recovery-code info log's "N codes remaining" detail.
- [x] 2.4 `auth.logout` alongside the audit row the handler already writes.
- [x] 2.5 `auth.sessions_invalidated` when the session-fixation sweep drops a
  member's sessions.

## 3. Rate limiting

- [x] 3.1 `auth.rate_limited` at every `login_limiter` and `money_limiter`
  rejection, naming the endpoint class and IP. No audit row.
- [x] 3.2 Confirm the limiter's own poisoned-mutex recovery path still logs its
  existing warning — do not replace it.

## 4. Password reset

- [x] 4.1 `auth.password_reset_requested`, including the unknown-address case with
  its reason. Response stays enumeration-safe and unchanged.
- [x] 4.2 Token consumption logs valid / already-used / expired distinctly —
  `src/web/templates/reset.rs`.
- [x] 4.3 `auth.password_reset_completed` plus an `audit_logs` row on success.

## 5. Password change and policy

- [x] 5.1 `auth.password_changed` plus an `audit_logs` row —
  `src/web/portal/profile.rs`.
- [x] 5.2 `auth.password_rejected` at every `validate_password` failure, carrying
  which rule failed and — for the length rules — the submitted length. Length is
  not sensitive; the password is. Cover all four call sites: setup, public signup,
  reset, profile.
- [x] 5.3 No audit row for a policy rejection (attacker-controlled volume).

## 6. Two-factor lifecycle

- [x] 6.1 `auth.totp_enrolled`, `auth.totp_disabled`,
  `auth.recovery_codes_regenerated`, each with an `audit_logs` row.

## 7. Tests

- [x] 7.1 Denial reasons differ in the log and not in the response (the core
  security assertion for this change).
- [x] 7.2 No log event contains a submitted password — assert against a captured
  subscriber, including the malformed-identifier redaction path.
- [x] 7.3 A burst of failed logins writes zero `audit_logs` rows.
- [x] 7.4 A password change and a TOTP disable each write exactly one audit row
  containing no credential material.
- [x] 7.5 Reset-token expiry, reuse, and validity produce three distinguishable
  log outcomes.

## 8. Documentation

- [x] 8.1 Note in the deploy docs that auth logs contain attempted email addresses
  and inherit the log store's retention and access controls — operators running
  this in a regulated context need to know before they ship logs off-box.
