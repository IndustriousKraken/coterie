# email-tokens Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Tokens are stored as SHA-256 hashes

`EmailTokenService` SHALL hash tokens with SHA-256 before persistence. The plaintext token SHALL exist only in the emailed URL and briefly in memory during request handling.

#### Scenario: Database row contains hash only

- **WHEN** a row is inserted into `email_verification_tokens` or `password_reset_tokens`
- **THEN** the `token_hash` column SHALL be `SHA-256(plaintext)`; the plaintext SHALL NOT be persisted

### Requirement: Tokens are single-use via atomic consume

`consume(token)` SHALL atomically flip `consumed_at` from NULL to the current time using a single UPDATE … RETURNING statement with `WHERE token_hash = ? AND consumed_at IS NULL AND expires_at > ?`. A second redemption attempt SHALL find no row matching all three conditions.

#### Scenario: Token redeems exactly once

- **WHEN** a token is consumed successfully
- **THEN** subsequent `consume` calls with the same token SHALL return `None` because `consumed_at` is now non-NULL

#### Scenario: Concurrent consume of the same token

- **WHEN** two concurrent requests attempt to consume the same valid token
- **THEN** the atomic UPDATE … RETURNING SHALL ensure exactly ONE caller receives the `ConsumedToken`; the other SHALL receive `None`

### Requirement: Expired tokens cannot redeem

The `WHERE` clause SHALL include `expires_at > now()` so an expired but unconsumed token SHALL NOT redeem.

#### Scenario: Expired token cannot redeem

- **WHEN** a token is presented after its `expires_at`
- **THEN** `consume` SHALL return `None` and any associated state change SHALL NOT occur

### Requirement: Cross-purpose protection is enforced by separate tables

The service SHALL be constructed for a specific purpose via `EmailTokenService::verification(pool)` or `EmailTokenService::password_reset(pool)`. Each constructor binds the service to its own table (`email_verification_tokens` or `password_reset_tokens`). A token from one table SHALL NOT be redeemable through the other table's service instance.

#### Scenario: Reset token cannot redeem at verification endpoint

- **WHEN** a token issued by `password_reset` is presented to the `verification` service's `consume`
- **THEN** the lookup SHALL find no row in `email_verification_tokens` and `consume` SHALL return `None`

### Requirement: Tokens are 256 bits of cryptographic randomness

Plaintext tokens SHALL be generated from 32 random bytes (256 bits) via the OS-RNG-backed `rand::thread_rng()`, hex-encoded for inclusion in URLs.

#### Scenario: Token value is unpredictable

- **WHEN** an attacker observes prior issued tokens
- **THEN** the next-issued token SHALL NOT be predictable; the keyspace is 2^256

### Requirement: Successful consume invalidates other outstanding tokens

`invalidate_for_member(member_id)` SHALL set `consumed_at` to the current time on every outstanding (unconsumed) token for that member. Reset/verification flows SHALL invoke this after a successful consume so other in-flight tokens for the same member become unusable.

#### Scenario: Other outstanding reset tokens are killed after a successful reset

- **WHEN** a member completes a password reset and the handler calls `invalidate_for_member`
- **THEN** any other outstanding reset tokens (e.g., a forgotten earlier request) SHALL be marked consumed and unusable

### Requirement: Background cleanup prunes expired rows

`cleanup_expired()` SHALL delete expired rows. Cleanup is best-effort; expiry is enforced at consume time regardless.

#### Scenario: Cleanup is not load-bearing for security

- **WHEN** `cleanup_expired` has not run for some time
- **THEN** expired tokens SHALL still be unusable because `consume` enforces the expiry condition independently

