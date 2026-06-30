# password-management Specification

## MODIFIED Requirements

### Requirement: Password complexity is validated at change/reset/signup

`crate::auth::validate_password` SHALL be invoked before hashing on every code path that sets a password (signup, in-portal change, reset). The validator's rules are the single source of truth for complexity. The validator SHALL enforce both a minimum length AND a maximum length: a password longer than the upper bound (128 characters) SHALL be rejected before it is Argon2-hashed, so an unauthenticated caller cannot force expensive hashing of an oversized input (an Argon2 CPU-amplification denial of service).

#### Scenario: Weak password rejected at change

- **WHEN** a member submits the password-change form with a password failing complexity rules
- **THEN** the handler SHALL render an inline error and SHALL NOT update the hash

#### Scenario: Weak password rejected at reset

- **WHEN** a reset-token consumer submits a password failing complexity rules
- **THEN** the handler SHALL reject the submission and the token SHALL NOT be marked consumed

#### Scenario: Over-long password rejected before hashing

- **WHEN** a password exceeding the maximum length (128 characters) is submitted on any set-password path (signup, reset, in-portal change, setup)
- **THEN** `validate_password` SHALL return an error and the password SHALL NOT be Argon2-hashed or persisted
