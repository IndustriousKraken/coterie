# member-dashboard Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Active/Honorary members see a personalized dashboard

`GET /portal/dashboard` SHALL render a member dashboard for Active and Honorary members. It SHALL show dues status, upcoming events, and recent activity. Expired members SHALL be redirected to `/portal/restore` by `require_auth_redirect`.

#### Scenario: Active member sees dashboard

- **WHEN** an Active member requests `/portal/dashboard`
- **THEN** the response SHALL render the dashboard with their personalized content

#### Scenario: Expired member is redirected to restoration

- **WHEN** an Expired member requests `/portal/dashboard`
- **THEN** the response SHALL be a redirect to `/portal/restore`

### Requirement: Dashboard partials are HTMX fragments

Sub-sections of the dashboard (upcoming events, recent payments, dues warning) SHALL be available as HTMX fragments at routes such as `/portal/api/events/upcoming`, `/portal/api/payments/recent`, `/portal/api/dues-warning`. Fragments SHALL return HTML, not JSON.

#### Scenario: Fragment endpoint returns HTML

- **WHEN** an authenticated request hits `/portal/api/events/upcoming` with the HTMX header
- **THEN** the response SHALL be an HTML fragment, not JSON

#### Scenario: Dues-warning fragment is reachable to Expired members

- **WHEN** an Expired member's restoration page renders the dues-warning banner
- **THEN** `/portal/api/dues-warning` SHALL be reachable under `require_restorable`

