# a45-auth-security-logging

## Why

On 2026-07-28 a member could not log in, and **nobody could tell why** — not the
member, not the operator. Coterie emits exactly one `tracing` event across its
entire authentication surface (a recovery-code login, in
`src/api/handlers/auth.rs`). There is no record of a failed login, a password
reset request, a rejected password, or a rate-limit trip.

That is a gap in two directions at once:

- **Operations.** "Can't get in" is unanswerable. Was the password wrong? Was the
  account expired? Did they hit the rate limiter? Did the reset email token expire
  before they used it? Today an operator can only guess, and guessing is how a
  ten-minute support question becomes an afternoon.
- **Security.** A credential-stuffing run, a targeted brute force, or a password
  spray against the member list is currently invisible. The rate limiter blocks it
  silently; no one learns it happened. For a deployment that runs a public bug
  bounty, being unable to distinguish an authorized tester from an actual attacker
  after the fact is a real deficiency.

### The gap was worse than "no auth events"

Investigating the 2026-07-29 lockout turned up the reason nothing was visible:
`TraceLayer` was applied inside `api::create_app()`, **before**
`api_app.merge(web_app)`. In axum 0.7 layers do not propagate across a merge — the
exact rule the file's own comments already state twice for CSRF. Tracing was never
moved, so the entire portal surface has produced **no request log at all**:
`uri=/login` appears zero times in twenty days of production logs.

Every one of the 174 rate-limit rejections in that period was invisible to the
application. The incident was only reconstructable from Caddy's access log. So
this change fixes the layer placement as well as adding the events — without the
placement fix, the new events would land in a surface that is still missing every
request-level status.

## What Changes

- **Every authentication-surface outcome emits a structured `tracing` event** with
  a consistent field set, so the log is greppable by event and by member rather
  than by prose.
- **Failure reasons are recorded in the log but never widened in the response.**
  The log distinguishes unknown-email from wrong-password from
  not-active-status; the HTTP response keeps saying the same enumeration-safe
  thing it says today. Making the log more specific than the response is the whole
  point — the operator needs the distinction and the anonymous caller must not
  have it.
- **Account-state-changing auth events also write `audit_logs` rows** — password
  changed, password reset completed, TOTP enabled/disabled, recovery codes
  regenerated — because those are reviewable account history, not just runtime
  telemetry. This follows the existing `audit-logging` pattern, which already
  covers logout.
- **High-volume, attacker-controlled events stay out of the database.** Failed
  logins, rate-limit trips and password-policy rejections are logs only. Writing a
  DB row per failed login hands an unauthenticated caller a write-amplification
  lever and buries real admin-reviewable history under noise.
- **Credentials are never logged.** The attempted identifier is logged because
  investigating without it is impossible; passwords, reset tokens, session tokens
  and TOTP codes are never logged at any level.

## Impact

- **Spec:** new capability `auth-logging` (6 ADDED requirements). MODIFIED:
  `audit-logging` (the locus-of-emission requirement gains the password/2FA
  lifecycle events alongside the logout entry it already lists).
- **Code (extend):** `src/api/handlers/auth.rs` and `src/web/templates/` login
  paths (login success/failure, TOTP, logout), `src/web/templates/reset.rs`
  (reset request, token consumption, completion), `src/web/portal/profile.rs`
  (password change, TOTP lifecycle), `src/auth/mod.rs` (policy rejection),
  and the rate-limiter call sites.
- **Reuse:** existing `tracing` setup and `AuditService`; no new dependency, no new
  table, no new endpoint.
- **Operational note:** these logs contain attempted email addresses, so they
  inherit whatever retention and access controls the deployment's log store
  already has. That is called out in the spec rather than left implicit.
- **Deferred:** shipping logs off-box, alerting thresholds ("N failures in M
  minutes"), and a portal-visible security-events view for members. Each is a real
  feature; none is needed to answer "why couldn't this person log in".
