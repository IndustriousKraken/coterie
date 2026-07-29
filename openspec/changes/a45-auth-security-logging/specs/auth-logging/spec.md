# auth-logging Specification

## ADDED Requirements

### Requirement: Request tracing covers the merged portal surface, not only the API

The HTTP request-tracing layer SHALL be applied to the fully merged router — after
`api_app.merge(web_app)` in `src/main.rs` — so that every route is traced,
including the portal and web-template routes brought in by the merge.

The tracing layer SHALL NOT be applied inside `api::create_app`. In axum 0.7 a
layer added before `Router::merge` does not propagate to the merged routes, so a
tracing layer placed there covers the API surface and silently omits the entire
portal: `/login`, `/forgot-password`, `/reset-password`, and every `/portal/*`
route. This is the same propagation rule that already forces CSRF to be applied
after the merge, and it SHALL be respected for the same reason.

The layer SHALL be applied OUTERMOST, wrapping the CSRF and setup-gate middleware,
so requests those layers reject are still logged rather than disappearing before
any handler runs.

A request that is rejected by a rate limiter, a CSRF check, or an authentication
guard SHALL produce a log line carrying its method, path, and status. An operator
investigating "this member cannot sign in" SHALL be able to answer it from the
application's own logs, without recourse to the reverse proxy's access log.

#### Scenario: A login attempt appears in the application log

- **WHEN** any request is made to `/login`, `/forgot-password`, `/reset-password`,
  or a `/portal/*` route
- **THEN** the application SHALL emit a request-scoped log line with the method,
  path, and response status

#### Scenario: A rate-limited request is logged rather than silently dropped

- **WHEN** a credential endpoint returns `429`
- **THEN** that response SHALL appear in the application log; an operator SHALL
  NOT have to read the reverse proxy's access log to discover that rate limiting
  fired

#### Scenario: Moving the layer inside the API router is a defect

- **WHEN** a contributor relocates the tracing layer into `api::create_app` or any
  position before the merge
- **THEN** that change SHALL be treated as a defect, because it removes request
  logging from every portal route while leaving the API surface working and so
  fails silently

### Requirement: The effective client-IP resolution mode is logged at startup

The application SHALL log, once at startup, how it will resolve the client IP used
to key rate limiting: whether `X-Forwarded-For` / `X-Real-Ip` are trusted, and
whether that decision was configured explicitly or inferred from the base URL's
scheme.

When the effective configuration would cause `client_ip` to fall back to the
localhost placeholder for ordinary requests — forwarded headers untrusted, with no
peer address available — the application SHALL emit a **warning** stating that
per-IP rate limiting has collapsed to a single shared bucket.

This matters because the collapse is otherwise undetectable from the outside and
its symptom is indistinguishable from correct behavior: every member shares one
five-attempts-per-fifteen-minutes budget, so a handful of failed logins anywhere
in the organization locks out everyone. An operator would see scattered reports of
"too many requests" from people who had barely tried, with nothing in the logs to
explain it.

The trust decision is currently **inferred** from whether the base URL is
`https://`, which is a reasonable default and a silent coupling: changing the base
URL scheme, or terminating TLS differently, would flip rate limiting from per-IP
to global without any other visible change. Logging the resolved mode makes that
coupling observable at the moment it matters.

#### Scenario: The resolved mode is visible at startup

- **WHEN** the application boots
- **THEN** it SHALL log whether forwarded headers are trusted and whether that came
  from explicit configuration or from scheme inference

#### Scenario: A collapsed bucket is warned about, not silently accepted

- **WHEN** the effective configuration would make every request resolve to the
  same placeholder IP
- **THEN** a warning SHALL be emitted naming the consequence — that per-IP rate
  limiting has become a single shared budget for all callers

### Requirement: Every authentication outcome emits a structured log event

Each outcome on the authentication surface SHALL emit exactly one `tracing` event
carrying a consistent, machine-filterable field set: an `event` name, an `outcome`
of `ok` or `denied`, the `member_id` when one is known, the caller's `ip`, and a
`reason` when the outcome is `denied`.

Fields SHALL be emitted as structured `tracing` fields rather than interpolated
into the message string, so an operator can filter on `event` and `member_id`
without pattern-matching prose.

The events SHALL be, at minimum:

- `auth.login` — first-factor password attempt, both surfaces (`/login`, `/auth/login`)
- `auth.totp` — second-factor TOTP or recovery-code attempt
- `auth.logout` — session ended by the user
- `auth.rate_limited` — a credential or money endpoint rejected a caller at the limit
- `auth.password_reset_requested` — a reset was asked for
- `auth.password_reset_completed` — a reset token was consumed and a new password set
- `auth.password_changed` — a logged-in member changed their own password
- `auth.password_rejected` — a submitted password failed the policy
- `auth.totp_enrolled` / `auth.totp_disabled` / `auth.recovery_codes_regenerated`
- `auth.sessions_invalidated` — every session for a member was swept

A successful login SHALL be logged at `info`; a denied outcome SHALL be logged at
`warn` so an operator scanning warnings sees the security-relevant subset without
reading every successful sign-in.

#### Scenario: A successful login is recorded with its member

- **WHEN** a member authenticates successfully
- **THEN** an `auth.login` event SHALL be emitted at `info` with `outcome = "ok"`,
  the member's id, and the caller's IP

#### Scenario: Events are filterable without parsing prose

- **WHEN** an operator wants every authentication denial for one member
- **THEN** filtering on the `event` and `member_id` structured fields SHALL be
  sufficient; the operator SHALL NOT need to match against message text

### Requirement: The log distinguishes failure reasons the response deliberately hides

A denied authentication SHALL record a specific `reason` in the log — at minimum
distinguishing `unknown_email`, `bad_password`, `inactive_status`,
`totp_invalid`, `totp_expired_pending`, and `rate_limited` — while the HTTP
response SHALL remain exactly as enumeration-safe as it is today.

This asymmetry is the point of the requirement, not an oversight: the operator
needs to tell "this person typed the wrong password" apart from "this account is
suspended" apart from "this email was never registered", and the anonymous caller
must be able to tell none of them apart. A change that makes the response more
specific in order to make the log more specific SHALL be rejected.

#### Scenario: An unknown email and a wrong password log differently but respond identically

- **WHEN** one caller submits an email that matches no member and another submits
  a known email with the wrong password
- **THEN** the two log events SHALL carry different `reason` values, and the two
  HTTP responses SHALL be indistinguishable in status, body, and timing class

#### Scenario: A suspended member's denial is visible to the operator

- **WHEN** a member with a non-active status authenticates with correct credentials
- **THEN** the log SHALL record `reason = "inactive_status"` with the member id, so
  the operator can answer "why can't they get in" without reproducing it

### Requirement: Credentials are never written to logs

Logs SHALL NOT contain passwords, password-reset tokens, session tokens, TOTP
codes, or recovery codes — at any level, in any field, whether the attempt
succeeded or failed.

The attempted **identifier** (email or username) SHALL be logged, because
investigating an access problem without knowing whose access failed is not
possible. Reset and session tokens MAY be referenced by a non-reversible short
prefix or an internal row id when correlation is genuinely needed, never in full.

A known hazard SHALL be respected when handling identifiers: users sometimes type
a password into the email field, so an identifier that failed to parse as an
email SHALL be logged as a redacted placeholder rather than verbatim, to avoid
capturing a credential in a log that outlives the request.

#### Scenario: A failed login does not record what was typed as the password

- **WHEN** any authentication attempt fails
- **THEN** no log event SHALL contain the submitted password in any form, hashed
  or otherwise

#### Scenario: A malformed identifier is redacted rather than echoed

- **WHEN** the submitted identifier is not a syntactically valid email address
- **THEN** the log SHALL record a redacted placeholder instead of the raw value,
  because the most likely cause is a password typed into the wrong field

### Requirement: Account-state-changing auth events are audited as well as logged

An `audit_logs` row SHALL be written — in addition to the log event — for each of
password change, completed password reset, TOTP enrolment, TOTP disablement, and
recovery-code regeneration, because these are reviewable account history rather
than runtime telemetry.

Each audit row SHALL identify the affected member as the entity and SHALL record
the acting member as actor, so an operator can later answer who changed what and
when. The audit row SHALL NOT contain the new credential or any part of it.

#### Scenario: A self-service password change is auditable after the fact

- **WHEN** a member changes their own password
- **THEN** an `audit_logs` row SHALL exist naming that member, alongside the
  `auth.password_changed` log event

#### Scenario: Disabling two-factor is reviewable

- **WHEN** a member disables TOTP on their account
- **THEN** an `audit_logs` row SHALL record it, so an operator reviewing an
  account compromise can see that second-factor protection was removed and when

### Requirement: Attacker-controlled events are logged but not persisted to the database

Failed logins, rate-limit rejections, and password-policy rejections SHALL emit
log events only and SHALL NOT write `audit_logs` rows.

The volume of these events is controlled by an unauthenticated caller. Writing a
database row per failed login would let an anonymous attacker drive unbounded
writes — a write-amplification lever against the same SQLite file that serves
every request — and would bury the admin-reviewable history the audit log exists
to provide under machine-generated noise.

The `audit_logs` table SHALL remain a record of things an operator would want to
read, and the log stream SHALL carry the things a machine generates.

#### Scenario: A brute-force run does not grow the audit table

- **WHEN** an attacker submits ten thousand failed logins
- **THEN** the `audit_logs` table SHALL be unchanged and the attempts SHALL be
  visible only in the log stream

#### Scenario: Rate-limit trips are visible without a database write

- **WHEN** a caller is rejected by the credential or money rate limiter
- **THEN** an `auth.rate_limited` event SHALL be emitted naming the endpoint class
  and the caller's IP, and no `audit_logs` row SHALL be written

### Requirement: Password-reset flow logs each stage distinctly

The password-reset flow SHALL emit a distinct event at each stage — request,
token consumption, and completion — so an operator can locate exactly where a
member's reset failed.

Token consumption SHALL record whether the token was valid, already used, or
expired. A reset requested for an address that matches no member SHALL be logged
with that reason while the HTTP response remains the same enumeration-safe
response it is today.

This granularity exists because "I never got in via the reset link" has several
distinct causes — mail never sent, link never clicked, token expired before use,
token already consumed — and they are indistinguishable to the member reporting
the problem.

#### Scenario: An expired reset token is diagnosable

- **WHEN** a member follows a reset link after the token has expired
- **THEN** an `auth.password_reset_completed` event SHALL be emitted with
  `outcome = "denied"` and a reason identifying expiry, distinguishable in the log
  from an invalid or already-consumed token

#### Scenario: A reset for an unknown address is logged but not revealed

- **WHEN** a reset is requested for an email matching no member
- **THEN** the log SHALL record the attempt and its reason, and the HTTP response
  SHALL be identical to the response for a known address
