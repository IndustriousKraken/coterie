## ADDED Requirements

### Requirement: Public signup creates a Pending member

`POST /public/signup` SHALL accept new-member signup data, create a member with status `Pending`, and trigger a verification email. The endpoint SHALL be CSRF-exempt and gated by:

1. CORS allowlist (only configured origins may call it from a browser).
2. Bot challenge (Turnstile-compatible verification).

Signup is NOT covered by `money_limiter` because no payment side-effect is initiated. The bot challenge is the abuse gate.

The endpoint SHALL be documented in `src/api/docs.rs` so the OpenAPI spec stays accurate.

#### Scenario: Successful signup returns 200 and emails verification

- **WHEN** valid signup data with a verified bot-challenge token reaches `/public/signup`
- **THEN** a new `Pending` member SHALL be persisted and a verification email SHALL be queued

#### Scenario: Missing bot token fails closed

- **WHEN** the bot-challenge provider is configured and the request omits the token
- **THEN** the request SHALL be rejected with 403 before signup logic runs

#### Scenario: Cross-origin from non-allowlisted origin is blocked

- **WHEN** a browser at a non-allowlisted origin attempts a cross-origin POST
- **THEN** the browser SHALL block it via the CORS policy

#### Scenario: Bot challenge runs before any database work

- **WHEN** a signup request reaches the handler with a missing or invalid token
- **THEN** the handler SHALL return 403 BEFORE any membership-type lookup or member creation, so an attacker cannot use signup to probe internal state

### Requirement: Pending members cannot log in until verified

A signup-created member with status `Pending` SHALL NOT pass `require_auth_redirect` or `require_auth`. The verification flow SHALL transition the member to a usable status when the email-token redeems.

#### Scenario: Pending member is rejected at login

- **WHEN** a Pending member completes the verification flow
- **THEN** their status SHALL transition to the configured initial active/expired state and login SHALL succeed

#### Scenario: Unverified Pending member cannot pass auth gates

- **WHEN** a Pending member somehow obtains a session
- **THEN** `require_auth` SHALL return 403 and `require_auth_redirect` SHALL bounce to login
