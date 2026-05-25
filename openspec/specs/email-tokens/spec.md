# email-tokens Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Tokens are stored as SHA-256 hashes

`EmailTokenService` SHALL hash tokens with SHA-256 before persistence. The
plaintext token SHALL exist only in the emailed URL and briefly in memory
during request handling.

#### Scenario: Stored hash equals SHA-256(plaintext)

- **WHEN** a token is created and the row inspected directly
- **THEN** `token_hash` SHALL equal `hex::encode(Sha256::digest(plaintext))`
  with no other transformation

### Requirement: Tokens are single-use via atomic consume

`consume(token)` SHALL atomically flip `consumed_at` from NULL to the current
time using a single UPDATE … RETURNING statement with `WHERE token_hash = ?
AND consumed_at IS NULL AND expires_at > ?`. A second redemption attempt
SHALL find no row matching all three conditions.

#### Scenario: Token redeems exactly once

- **WHEN** a token is consumed successfully
- **THEN** subsequent `consume` calls with the same token SHALL return
  `None` because `consumed_at` is now non-NULL

#### Scenario: Consume of an unknown plaintext returns None without error

- **WHEN** `consume("not-a-real-token")` is called
- **THEN** it SHALL return `Ok(None)` (no row matched the SHA-256 hash)

### Requirement: Expired tokens cannot redeem

The `WHERE` clause SHALL include `expires_at > now()` so an expired but
unconsumed token SHALL NOT redeem.

#### Scenario: Expired token cannot redeem

- **WHEN** a token is presented after its `expires_at`
- **THEN** `consume` SHALL return `None` AND the row's `consumed_at` SHALL
  remain NULL (the gated UPDATE did nothing)

### Requirement: Tokens are 256 bits of cryptographic randomness

Plaintext tokens SHALL be generated from 32 random bytes (256 bits) via the OS-RNG-backed `rand::thread_rng()`, hex-encoded for inclusion in URLs.

#### Scenario: Token value is unpredictable

- **WHEN** an attacker observes prior issued tokens
- **THEN** the next-issued token SHALL NOT be predictable; the keyspace is 2^256

### Requirement: Successful consume invalidates other outstanding tokens

`invalidate_for_member(member_id)` SHALL set `consumed_at` to the current
time on every outstanding (unconsumed) token for that member.
Reset/verification flows SHALL invoke this after a successful consume so
other in-flight tokens for the same member become unusable.

#### Scenario: Other outstanding reset tokens are killed after invalidate

- **WHEN** a member has two outstanding reset tokens and the handler calls
  `invalidate_for_member`
- **THEN** both subsequent `consume` calls for those tokens SHALL return
  `None`

### Requirement: Background cleanup prunes expired rows

`cleanup_expired()` SHALL delete expired rows. Cleanup is best-effort;
expiry is enforced at consume time regardless.

#### Scenario: Cleanup removes only expired rows

- **WHEN** the table holds one expired and one unexpired row, and
  `cleanup_expired()` is called
- **THEN** the call SHALL return 1 (rows deleted) AND the unexpired token
  SHALL still successfully redeem via `consume`

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

