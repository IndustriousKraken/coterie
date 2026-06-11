## RENAMED Requirements

- FROM: `### Requirement: Cross-purpose protection is enforced by separate tables`
- TO: `### Requirement: Cross-purpose protection is enforced by separate tables and kind-specific functions`

## MODIFIED Requirements

### Requirement: Cross-purpose protection is enforced by separate tables and kind-specific functions

The system SHALL expose four free functions for token operations, one pair per kind:

- `create_verification_token(pool, member_id, ttl)` / `consume_verification_token(pool, token)` — bound to `email_verification_tokens`.
- `create_password_reset_token(pool, member_id, ttl)` / `consume_password_reset_token(pool, token)` — bound to `password_reset_tokens`.

A `pub struct EmailTokenService` with factory constructors SHALL NOT exist; ServiceContext SHALL NOT carry an Arc'd token-service instance. The free-function shape matches the actual call-site pattern (per-request issuance/consumption in pre-auth handlers) and removes the abstraction mismatch the prior shape carried.

Cross-purpose protection (a verification token cannot be redeemed as a password-reset token, and vice versa) SHALL still hold because each public function statically passes its kind-specific table name to the private SQL helpers; no dynamic dispatch on kind exists at the call site.

#### Scenario: Free-function API is the only path

- **WHEN** a contributor needs to issue or consume an email token
- **THEN** they SHALL call one of the four free functions in `auth::email_tokens`; no struct instance is constructed and no ServiceContext field is consulted

#### Scenario: Cross-table redemption is impossible

- **WHEN** a verification token is presented to `consume_password_reset_token`
- **THEN** the call SHALL return `None` because the SELECT runs against `password_reset_tokens` only; the token's row lives in `email_verification_tokens` and is not consulted
