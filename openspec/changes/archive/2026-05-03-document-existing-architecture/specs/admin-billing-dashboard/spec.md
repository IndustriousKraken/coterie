## ADDED Requirements

### Requirement: Read-only billing dashboard

`GET /portal/admin/billing/dashboard` SHALL render a read-only summary of upcoming charges, recent failures, and revenue by month. The dashboard SHALL NOT expose state-changing actions; per-member actions live on `/portal/admin/members/:id/...`.

#### Scenario: Dashboard renders without state changes

- **WHEN** an admin visits the dashboard
- **THEN** no payment SHALL be initiated, retried, or modified by the page render

#### Scenario: Failures view groups by member

- **WHEN** the dashboard renders recent failures
- **THEN** the failures SHALL be grouped or linked per member so the admin can drill into the per-member page to act

### Requirement: Bulk Stripe-subscription migration is opt-in

`/portal/admin/settings/billing` SHALL expose a bulk action `POST /portal/admin/settings/billing/migrate-stripe-subs` to migrate members from Stripe-managed subscriptions to Coterie-managed billing. The action SHALL be CSRF-protected, admin-only, and atomic per member.

#### Scenario: Bulk migration is auditable

- **WHEN** the bulk migration completes
- **THEN** an audit-log entry SHALL be written summarizing the count migrated and any per-member failures
