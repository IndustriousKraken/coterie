# recovery-codes Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Ten codes per enrollment, displayed once

The system SHALL generate exactly 10 recovery codes at enrollment confirmation and at regenerate. The plaintext codes SHALL be returned to the caller for one-time display; only their hashes SHALL be persisted.

#### Scenario: Plaintext is not retrievable after issuance

- **WHEN** a member completes enrollment and receives the codes
- **THEN** the database SHALL store only argon2 hashes; the plaintext SHALL NOT be retrievable from the system thereafter

### Requirement: Codes are random from a look-alike-free alphabet

Each code SHALL be 12 characters drawn from the alphabet `ABCDEFGHJKMNPQRSTVWXYZ23456789` (no `0/O`, no `1/I/L`), formatted for display as three hyphen-separated groups of 4 (`XXXX-XXXX-XXXX`). The keyspace SHALL be 30^12 ≈ 5.3 × 10^17.

#### Scenario: Code format is XXXX-XXXX-XXXX

- **WHEN** a fresh batch of codes is generated
- **THEN** each plaintext code SHALL match `^[A-Z2-9]{4}-[A-Z2-9]{4}-[A-Z2-9]{4}$` and contain no look-alike characters

### Requirement: Hashes are argon2; verification normalizes input

Each plaintext code SHALL be normalized (whitespace + hyphens stripped, uppercased) before hashing with argon2. Verification SHALL normalize the user's submitted value the same way before checking. Hyphens and case in the user's input SHALL NOT affect the match.

#### Scenario: Lowercase paste with spaces matches stored hash

- **WHEN** a user pastes "abcd efgh jkmn" for an issued code "ABCD-EFGH-JKMN"
- **THEN** verification SHALL succeed because both normalize to "ABCDEFGHJKMN"

### Requirement: Storage is JSON array of hashes on the members row

Hashes SHALL be persisted as a JSON-encoded array in `members.totp_recovery_codes`. There SHALL be no separate recovery-codes table.

#### Scenario: Member row holds the JSON blob

- **WHEN** a member's row is read
- **THEN** `totp_recovery_codes` SHALL contain a JSON array of argon2 hash strings (or NULL if TOTP is not enabled)

### Requirement: Consume walks all entries (constant-time iteration)

`try_consume(member_id, submitted)` SHALL iterate every stored hash even after a match is found, recording the first matching index but not exiting the loop early. Time-cost SHALL be constant regardless of which code (or no code) was submitted.

#### Scenario: Verification time does not depend on which code matched

- **WHEN** a user submits the 1st code vs the 10th code
- **THEN** the verification SHALL take essentially the same wall time (10 argon2 verifications either way)

### Requirement: Successful consume removes only the matched entry, atomically

When a match is found, the matched hash SHALL be removed from the array, the JSON re-serialized, and persisted within the same transaction as the SELECT. Other hashes SHALL remain. Concurrent calls with the same code SHALL NOT both succeed.

#### Scenario: Code redeems exactly once under concurrency

- **WHEN** two concurrent requests submit the same valid recovery code
- **THEN** SQLite's per-row write lock plus the in-transaction read-then-rewrite SHALL ensure only ONE call returns `true`; the other SHALL see the rewritten JSON and return `false`

#### Scenario: Other codes survive a consume

- **WHEN** one of ten codes is consumed
- **THEN** the remaining 9 hashes SHALL still be valid and the array length SHALL drop to 9

### Requirement: Member can regenerate the code set

`POST /portal/profile/security/totp/recovery-codes/regenerate` SHALL call `issue_for_member`, which overwrites the row's `totp_recovery_codes` with a fresh batch and returns the plaintext for one-time display. Previous codes SHALL no longer redeem.

#### Scenario: Regeneration invalidates old codes

- **WHEN** a member regenerates codes
- **THEN** the previous 10 hashes SHALL be replaced; any leftover plaintext from the old batch SHALL no longer redeem

#### Scenario: Regeneration is CSRF-protected

- **WHEN** a regeneration request arrives without a valid CSRF token
- **THEN** the top-level CSRF middleware SHALL reject it with 403

### Requirement: Remaining count is visible to the member

`remaining_count(member_id)` SHALL return how many unconsumed codes remain so the security page can warn the member when the count is low.

#### Scenario: Count drops as codes are consumed

- **WHEN** a member consumes one of their codes
- **THEN** `remaining_count` SHALL return one less than before

