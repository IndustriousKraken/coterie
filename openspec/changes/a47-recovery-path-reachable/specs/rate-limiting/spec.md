# rate-limiting Specification

## MODIFIED Requirements

### Requirement: Credential flows are rate-limited per IP

The system SHALL apply a per-IP rate limit (`login_limiter`) of 5 attempts per 15 minutes to credential-handling endpoints. The current callers are:

- `POST /auth/login` (api handler) — first-factor password attempt.
- `POST /login` (web-template handler) — first-factor password attempt.
- `POST /auth/login/totp` (api handler) — second-factor TOTP/recovery-code attempt.
- `POST /login/totp` (web handler) — second-factor TOTP/recovery-code attempt.

The limiter SHALL be stored on `AppState` and shared across all surfaces so the same IP cannot get a fresh budget by hitting parallel paths. An attacker who exhausts the budget on `/auth/login` cannot switch to `/auth/login/totp` (or vice versa) to get a fresh allowance — the budget is one shared bucket per IP across these credential endpoints.

This design accepts a minor UX cost: a legitimate user who fails 5 password attempts and then succeeds may have 0 remaining attempts for the TOTP step. The 15-minute window auto-recovers; documented as an acceptable trade-off because the attack model (post-password brute-force on the 6-digit TOTP) is the dominant concern.

**Password recovery SHALL NOT share this budget.** `POST /forgot-password` SHALL be limited by a separate `recovery_limiter` with its own per-IP bucket, so exhausting the credential budget does not also close the recovery path.

The reason is that the two budgets protect against different things and coupling them produces a trap: the user most likely to need a password reset is the user who has just failed several logins, and under a shared budget that is exactly the user who can no longer request one. Forgetting a password then locks the member out of the mechanism built for forgotten passwords, and the only escape is to wait out a window nothing tells them about. This is not hypothetical — a member hit it in production on 2026-07-29, exhausting the budget on five failed logins and then receiving `429` on `/forgot-password`.

Recovery SHALL still be limited, because it sends email and is therefore abusable as an amplification vector; it simply SHALL be limited independently.

#### Scenario: Sixth login attempt is rejected

- **WHEN** the same IP attempts to log in 6 times within a 15-minute window
- **THEN** the 6th attempt SHALL be rejected with `429 Too Many Requests`

#### Scenario: Reset does NOT share the budget with login

- **WHEN** an IP exhausts the login limit and then immediately attempts a password-reset request
- **THEN** the reset SHALL be evaluated against `recovery_limiter` and SHALL be accepted if that separate budget has room; a member locked out of login SHALL still be able to request a reset

#### Scenario: Limit resets after the window

- **WHEN** the same IP waits 15 minutes after exhausting the limit
- **THEN** subsequent attempts SHALL be accepted again up to the budget

#### Scenario: TOTP second-factor attempts share the credential-flow budget

- **WHEN** an IP submits 5 wrong TOTP codes to `/auth/login/totp` (or `/login/totp`) within a 15-minute window
- **THEN** the 6th submission SHALL be rejected with `429 Too Many Requests`; this prevents a stolen-password attacker from brute-forcing the 6-digit TOTP code space

#### Scenario: Switching surfaces does not multiply the budget

- **WHEN** an IP exhausts the budget by failing 5 password attempts on `/auth/login` and then attempts `POST /auth/login/totp`
- **THEN** the TOTP attempt SHALL ALSO be rejected with `429`; the shared `login_limiter` does not give a fresh allowance for the second-factor endpoint

#### Scenario: Recovery is still limited on its own bucket

- **WHEN** an IP submits password-reset requests beyond the `recovery_limiter` budget
- **THEN** the excess requests SHALL be rejected with `429`; separating the budget SHALL NOT mean leaving recovery unlimited, because each request sends email
