# public-signup Specification

## MODIFIED Requirements

### Requirement: Public signup creates a Pending member

`POST /public/signup` SHALL accept new-member signup data, create a member with status `Pending`, and trigger a verification email. The endpoint SHALL be CSRF-exempt and gated by:

1. CORS allowlist (only configured origins may call it from a browser).
2. `money_limiter` (per-IP rate limit, applied in BOTH signup modes).
3. Bot challenge (Turnstile-compatible verification).

`money_limiter` SHALL run BEFORE the bot-challenge provider in both modes so a bursting IP cannot burn the provider's quota. When the organization's signup mode is `approval` (the default), signup initiates no payment side-effect but SHALL still be covered by `money_limiter` to cap mass account creation and verification-email amplification. When the signup mode is `payment`, signup initiates a payment side-effect and the same `money_limiter` applies per the money-moving public-endpoint gate order.

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

#### Scenario: Approval-mode signup is rate-limited

- **WHEN** the org's signup mode is `approval` and an IP at the money-limiter budget submits another signup
- **THEN** the request SHALL be rejected with `429` WITHOUT consulting the bot-challenge provider
