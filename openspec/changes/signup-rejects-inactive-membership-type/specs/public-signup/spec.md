# public-signup Specification

## ADDED Requirements

### Requirement: Signup rejects unknown or inactive membership types

`POST /public/signup` SHALL reject a supplied `membership_type_slug` that does not resolve to an ACTIVE membership type. An unknown slug SHALL be rejected with `400`; a slug that resolves to a membership type whose `is_active` flag is false SHALL ALSO be rejected with `400`, before any member is created — a deactivated type is not signup-able even though it still exists in the database. An omitted slug SHALL take the organization's default (the first active membership type by sort order), and a known, active slug SHALL be accepted unchanged.

#### Scenario: Inactive membership-type slug is rejected

- **WHEN** a signup supplies a `membership_type_slug` that exists but whose type is inactive
- **THEN** the handler SHALL return `400` and SHALL NOT create a member

#### Scenario: Unknown membership-type slug is rejected

- **WHEN** a signup supplies a `membership_type_slug` that matches no membership type
- **THEN** the handler SHALL return `400` and SHALL NOT create a member

#### Scenario: Omitted slug takes the org default

- **WHEN** a signup omits `membership_type_slug`
- **THEN** the member SHALL be created on the organization's default (first active) membership type
