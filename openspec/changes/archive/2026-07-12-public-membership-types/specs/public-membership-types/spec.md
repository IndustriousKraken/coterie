# public-membership-types Specification

## ADDED Requirements

### Requirement: Public endpoint lists active membership types

`GET /public/membership-types` SHALL return the organization's active membership types as JSON — for each type: `slug`, `name`, `description`, `fee_cents`, `currency`, and `billing_period` — ordered by `sort_order`. Inactive types SHALL be excluded. The endpoint SHALL be unauthenticated and read-only, gated the same way as the other public read endpoints (CORS allowlist for browsers; no bot challenge or money limiter), and SHALL be documented in the OpenAPI spec in `src/api/docs.rs`.

#### Scenario: Active types are listed for the public join form

- **WHEN** an unauthenticated GET reaches `/public/membership-types` and two active types exist
- **THEN** the response SHALL be `200` with both types including `slug`, `name`, `description`, `fee_cents`, `currency`, and `billing_period`, ordered by `sort_order`

#### Scenario: Inactive types are excluded

- **WHEN** a membership type is marked inactive
- **THEN** it SHALL NOT appear in the `/public/membership-types` response

#### Scenario: No types configured returns an empty list

- **WHEN** no active membership types exist
- **THEN** the response SHALL be `200` with an empty JSON array
