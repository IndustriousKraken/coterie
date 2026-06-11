# member-donations Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Logged-in members can donate via the portal

`GET /portal/donate` SHALL render a donation page; `POST /portal/api/donate` SHALL initiate the donation flow for the logged-in member. Both routes SHALL be gated by `require_auth_redirect` (Active/Honorary only).

#### Scenario: Logged-in donation pre-fills member info

- **WHEN** an Active member opens `/portal/donate`
- **THEN** the donation form SHALL pre-fill name and email from the member's record

#### Scenario: Successful donation routes through Stripe

- **WHEN** a member submits the donation form
- **THEN** the system SHALL initiate a Stripe payment using the member's saved card (or Checkout for a new card) and emit an audit-log entry on completion via the webhook flow

