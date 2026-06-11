## MODIFIED Requirements

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

### Requirement: Cross-purpose protection is enforced by separate tables

The service SHALL be constructed for a specific purpose via
`EmailTokenService::verification(pool)` or
`EmailTokenService::password_reset(pool)`. Each constructor binds the service
to its own table (`email_verification_tokens` or `password_reset_tokens`). A
token from one table SHALL NOT be redeemable through the other table's
service instance.

#### Scenario: Reset token cannot redeem at verification endpoint

- **WHEN** a token issued by `password_reset` is presented to the
  `verification` service's `consume`
- **THEN** the lookup SHALL find no row in `email_verification_tokens` and
  `consume` SHALL return `None`

#### Scenario: Verification token cannot redeem at password-reset endpoint

- **WHEN** a token issued by `verification` is presented to the
  `password_reset` service's `consume`
- **THEN** the lookup SHALL find no row in `password_reset_tokens` and
  `consume` SHALL return `None`

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

### Requirement: Tokens are stored as SHA-256 hashes

`EmailTokenService` SHALL hash tokens with SHA-256 before persistence. The
plaintext token SHALL exist only in the emailed URL and briefly in memory
during request handling.

#### Scenario: Stored hash equals SHA-256(plaintext)

- **WHEN** a token is created and the row inspected directly
- **THEN** `token_hash` SHALL equal `hex::encode(Sha256::digest(plaintext))`
  with no other transformation
