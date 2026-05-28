# rate-limiting Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Credential flows are rate-limited per IP

The system SHALL apply a per-IP rate limit (`login_limiter`) of 5 attempts per 15 minutes to credential-handling endpoints. The current callers are:

- `POST /auth/login` (api handler) — first-factor password attempt.
- `POST /login` (web-template handler) — first-factor password attempt.
- `POST /auth/login/totp` (api handler) — second-factor TOTP/recovery-code attempt.
- `POST /login/totp` (web handler) — second-factor TOTP/recovery-code attempt.
- `POST /forgot-password` (password-reset request).

The limiter SHALL be stored on `AppState` and shared across all surfaces so the same IP cannot get a fresh budget by hitting parallel paths. An attacker who exhausts the budget on `/auth/login` cannot switch to `/auth/login/totp` (or vice versa) to get a fresh allowance — the budget is one shared bucket per IP across all five endpoints.

This design accepts a minor UX cost: a legitimate user who fails 5 password attempts and then succeeds may have 0 remaining attempts for the TOTP step. The 15-minute window auto-recovers; documented as an acceptable trade-off because the attack model (post-password brute-force on the 6-digit TOTP) is the dominant concern.

#### Scenario: Sixth login attempt is rejected

- **WHEN** the same IP attempts to log in 6 times within a 15-minute window
- **THEN** the 6th attempt SHALL be rejected with `429 Too Many Requests`

#### Scenario: Reset shares the budget with login

- **WHEN** an IP exhausts the login limit and then immediately attempts a password-reset request
- **THEN** the reset SHALL ALSO be rejected; both endpoints share `login_limiter`

#### Scenario: Limit resets after the window

- **WHEN** the same IP waits 15 minutes after exhausting the limit
- **THEN** subsequent attempts SHALL be accepted again up to the budget

#### Scenario: TOTP second-factor attempts share the credential-flow budget

- **WHEN** an IP submits 5 wrong TOTP codes to `/auth/login/totp` (or `/login/totp`) within a 15-minute window
- **THEN** the 6th submission SHALL be rejected with `429 Too Many Requests`; this prevents a stolen-password attacker from brute-forcing the 6-digit TOTP code space

#### Scenario: Switching surfaces does not multiply the budget

- **WHEN** an IP exhausts the budget by failing 5 password attempts on `/auth/login` and then attempts `POST /auth/login/totp`
- **THEN** the TOTP attempt SHALL ALSO be rejected with `429`; the shared `login_limiter` does not give a fresh allowance for the second-factor endpoint

### Requirement: Money-moving endpoints are rate-limited per IP

The system SHALL apply a per-IP rate limit (`money_limiter`) of 10 requests per 60 seconds to money-moving endpoints. Current callers:

- `POST /public/donate` — public donation flow.
- `POST /portal/api/payments/checkout`, `POST /portal/api/payments/charge-saved` — portal-initiated payments.
- `POST /portal/donate` API — logged-in donations.
- `POST /portal/admin/members/:id/record-payment` — admin manual payment recording.

Adding a money-moving endpoint without wiring `money_limiter` SHALL be treated as a defect. Note: `/public/signup` does NOT use this limiter (signup creates a Pending member with no payment side-effect); its abuse-control is bot challenge + CORS only.

#### Scenario: Donation flood is rejected

- **WHEN** an IP submits 11 donation requests within 60 seconds
- **THEN** the 11th request SHALL be rejected by the rate limiter

#### Scenario: New money endpoint must subscribe to the limiter

- **WHEN** a new endpoint that records or initiates a payment is added
- **THEN** it SHALL invoke the shared `money_limiter` and be added to the rate-limited set; reviewers SHALL block PRs that omit this

### Requirement: Rate-limiter mutex poisoning is recoverable

The in-memory rate limiter SHALL recover gracefully if its internal mutex becomes poisoned (e.g., due to a panic in another thread). The limiter SHALL log a warning and continue serving rather than propagating the panic.

#### Scenario: Poisoned mutex logs and recovers

- **WHEN** a thread panics while holding the rate-limiter mutex
- **THEN** subsequent calls SHALL log "RateLimiter mutex was poisoned; recovering" and continue best-effort

### Requirement: Periodic cleanup runs in a background task

The application SHALL spawn a background task per limiter to periodically purge expired buckets and prevent unbounded memory growth. The cadence SHALL match each limiter's window (login: ~15 min; money: ~1 min).

#### Scenario: Cleanup task runs continuously

- **WHEN** the application is running
- **THEN** background tasks SHALL invoke `limiter.cleanup()` on a regular cadence so the in-memory map does not grow without bound

