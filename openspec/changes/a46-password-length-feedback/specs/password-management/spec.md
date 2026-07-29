# password-management Specification

## MODIFIED Requirements

### Requirement: Password complexity is validated at change/reset/signup

`crate::auth::validate_password` SHALL be invoked before hashing on every code path that sets a password (signup, in-portal change, reset, setup). The validator's rules are the single source of truth for complexity. The validator SHALL enforce both a minimum length AND a maximum length: a password longer than the upper bound (128 **bytes**) SHALL be rejected before it is Argon2-hashed, so an unauthenticated caller cannot force expensive hashing of an oversized input (an Argon2 CPU-amplification denial of service).

The bound SHALL be denominated in **bytes**, and every message describing it SHALL say bytes rather than characters. The check measures UTF-8 byte length, which is the quantity the denial-of-service argument is actually about — Argon2's pre-hash cost scales with bytes fed to it, not with Unicode scalar values. Describing a byte limit as a character limit is not a harmless simplification: a 60-character password made of emoji is 240 bytes, so a user is told they exceeded "128 characters" when they typed 60. Someone hitting that message reasonably concludes the system is broken.

An over-length rejection SHALL state both the ceiling and the size of what was submitted, so the user can act on it rather than guess how much to remove. The same rule SHALL apply to the minimum-length message, which carries the same ambiguity.

The password SHALL NOT be silently truncated to fit. Truncation would leave the account with a credential that is a prefix of what the user believes they set — indistinguishable, from the user's side, from the account being broken.

Password inputs SHALL carry a `maxlength` attribute matching the bound and a visible hint stating it, so the ceiling is discoverable before submission rather than only by tripping it. These client-side attributes are a convenience only: the server-side check remains authoritative and SHALL NOT be weakened or skipped on the assumption that the browser enforced it.

#### Scenario: Weak password rejected at change

- **WHEN** a member submits the password-change form with a password failing complexity rules
- **THEN** the handler SHALL render an inline error and SHALL NOT update the hash

#### Scenario: Weak password rejected at reset

- **WHEN** a reset-token consumer submits a password failing complexity rules
- **THEN** the handler SHALL reject the submission and the token SHALL NOT be marked consumed

#### Scenario: Over-long password rejected before hashing

- **WHEN** a password exceeding the maximum length (128 bytes) is submitted on any set-password path (signup, reset, in-portal change, setup)
- **THEN** `validate_password` SHALL return an error and the password SHALL NOT be Argon2-hashed or persisted

#### Scenario: A multi-byte password is measured and described in the same unit

- **WHEN** a user submits a password of 60 emoji characters, which is 240 UTF-8 bytes
- **THEN** it SHALL be rejected, and the message SHALL describe the limit in bytes and report the submitted byte size; it SHALL NOT claim the user exceeded a 128-**character** limit

#### Scenario: An over-long password is never truncated to fit

- **WHEN** a password longer than the bound is submitted on any set-password path
- **THEN** the submission SHALL be refused outright; no prefix of it SHALL be hashed or stored, and the account's existing credential SHALL be left unchanged

#### Scenario: The ceiling is discoverable before submission

- **WHEN** a user focuses a password field on any set-password form
- **THEN** the field SHALL carry a `maxlength` matching the bound and a visible hint stating it, so the limit is known before the form is submitted
