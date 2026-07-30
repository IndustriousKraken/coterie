# password-management Specification

## MODIFIED Requirements

### Requirement: Password complexity is validated at change/reset/signup

`crate::auth::validate_password` SHALL be invoked before hashing on every code path that sets a password (signup, in-portal change, reset, setup). The validator's rules are the single source of truth for complexity. The validator SHALL enforce both a minimum length AND a maximum length: a password longer than the upper bound (128 **bytes**) SHALL be rejected before it is Argon2-hashed, so an unauthenticated caller cannot force expensive hashing of an oversized input (an Argon2 CPU-amplification denial of service).

The bound SHALL be denominated in **bytes**, and every message describing it SHALL say bytes rather than characters. The check measures UTF-8 byte length, which is the quantity the denial-of-service argument is actually about — Argon2's pre-hash cost scales with bytes fed to it, not with Unicode scalar values. Describing a byte limit as a character limit is not a harmless simplification: a 60-character password made of emoji is 240 bytes, so a user is told they exceeded "128 characters" when they typed 60. Someone hitting that message reasonably concludes the system is broken.

An over-length rejection SHALL state both the ceiling and the size of what was submitted, so the user can act on it rather than guess how much to remove. The same rule SHALL apply to the minimum-length message, which carries the same ambiguity.

The password SHALL NOT be silently truncated to fit. Truncation would leave the account with a credential that is a prefix of what the user believes they set — indistinguishable, from the user's side, from the account being broken.

Password inputs SHALL carry a visible hint stating the ceiling, so it is discoverable before submission rather than only by tripping it. The hint is a convenience only: the server-side check remains authoritative and SHALL NOT be weakened or skipped on the assumption that the browser enforced it.

Password inputs SHALL NOT carry a `maxlength` attribute. Two independent reasons:

1. **It silently truncates.** A browser clips pasted input to `maxlength` without notifying the user. Someone pasting a 200-character password from a password manager would have 128 characters accepted and stored, believe they set the longer one, and be unable to sign in afterwards — reintroducing at the client exactly the silent-truncation failure this requirement forbids at the server. For a field whose contents are masked, the user cannot even see that it happened.
2. **The units do not match.** `maxlength` counts UTF-16 code units, while the bound is bytes. `maxlength="128"` would admit 128 characters, which can be several hundred bytes, so it cannot express the server's rule even approximately for non-ASCII input.

Client-side feedback, where provided, SHALL be non-destructive: it MAY warn that the entered value exceeds the ceiling — measuring UTF-8 byte length, e.g. via `TextEncoder`, to match the server — but SHALL NOT alter, clip, or reject the user's input. The server's rejection message is what tells the user authoritatively.

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
- **THEN** a visible hint SHALL state the ceiling, so the limit is known before the form is submitted

#### Scenario: A pasted over-length password is not silently clipped

- **WHEN** a user pastes a 200-character password into any password field
- **THEN** the full value SHALL remain in the field and be submitted as typed; no `maxlength` SHALL clip it to the bound, because a masked field gives the user no way to notice the loss and they would be locked out by a credential they never chose

#### Scenario: Client-side feedback measures the same unit as the server

- **WHEN** a client-side warning about length is shown
- **THEN** it SHALL measure UTF-8 byte length so it agrees with the server's rule, and SHALL NOT modify the entered value
