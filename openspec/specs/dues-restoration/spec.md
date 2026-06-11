# dues-restoration Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Expired members reach a narrow restoration scope

Members with status `Expired` SHALL be allowed to access only the routes needed to view dues, manage payment methods, pay, and view historical receipts. These routes SHALL be the entire set guarded by `require_restorable`:

- `GET /portal/restore` — restoration landing page.
- `GET /portal/payments/new`, `methods`, `success`, `cancel` — payment pages.
- `GET /portal/payments/receipts` and `/portal/payments/:payment_id/receipt`.
- `GET/POST /portal/api/payments/...` — payment fragments and saved-card management within the restoration scope.

All other portal routes SHALL be gated by `require_auth_redirect` and SHALL redirect Expired members back to `/portal/restore`.

#### Scenario: Expired member can pay to restore

- **WHEN** an Expired member completes a successful dues payment
- **THEN** the webhook flow SHALL transition them to Active and update `dues_paid_until`

#### Scenario: Expired member is bounced from non-restorable routes

- **WHEN** an Expired member requests `/portal/events`
- **THEN** the response SHALL be a redirect to `/portal/restore`

#### Scenario: Expired member can view historical receipts

- **WHEN** an Expired member visits `/portal/payments/receipts`
- **THEN** the page SHALL render their full receipt history; this is restorable scope so members can pull receipts for tax filing even when not currently Active

### Requirement: Restoration flow updates auto-renew preference based on member action

If the member opts in to auto-renew during restoration, the system SHALL set the member's auto-renew preference and require a saved card. Opting out SHALL clear any auto-renew schedule.

#### Scenario: Opting in requires a saved card

- **WHEN** a restoring member submits the form with auto-renew enabled but no saved card
- **THEN** the handler SHALL prompt them to add a card before completing the toggle

