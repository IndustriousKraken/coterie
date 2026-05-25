# admin-payments Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Admin records manual payments via the payment service

Manual payment recording SHALL go through the payment service so that audit-log entries and integration events match the Stripe-webhook path. Admins SHALL access this via `GET/POST /portal/admin/members/:id/record-payment`.

#### Scenario: Manual recording emits the same side effects as a webhook payment

- **WHEN** an admin records a manual cash payment
- **THEN** the resulting audit-log row, integration events, and dues-paid-until update SHALL match those that the corresponding Stripe-webhook path would emit

### Requirement: Refunds are recorded against the original payment

`POST /portal/admin/payments/:id/refund` SHALL record a refund against the original payment row and trigger the corresponding Stripe API call (or skip it for manual-payment refunds, recording only). The service SHALL emit an audit-log row.

#### Scenario: Refund without Stripe id records ledger entry only

- **WHEN** an admin refunds a manual (non-Stripe) payment
- **THEN** the system SHALL record the refund in the ledger and emit an audit-log entry without calling Stripe

#### Scenario: Refund with Stripe id calls Stripe and records the response

- **WHEN** an admin refunds a Stripe-backed payment
- **THEN** the system SHALL call Stripe to issue the refund, record the resulting refund id, and emit an audit-log entry

### Requirement: Refund handler lives in admin/payments.rs and routes through PaymentAdminService

The `admin_refund_payment` handler SHALL live in `src/web/portal/admin/payments.rs`, not in `src/web/portal/admin/members.rs`. The file location matches the URL path (`/portal/admin/payments/:id/refund`).

The handler SHALL parse the URL path parameter and the IP from headers, then call `PaymentAdminService::refund(current_user.id, payment_id, ip)`. The handler SHALL render `refund_result_html` based on the typed `Result<RefundOutcome, RefundError>` returned. Handler body SHALL be on the order of 25 lines; the orchestration chain is in the service, not here.

#### Scenario: Refund handler file location matches URL

- **WHEN** a contributor looks for the handler serving `POST /portal/admin/payments/:id/refund`
- **THEN** they SHALL find it in `src/web/portal/admin/payments.rs`, not in `members.rs`

#### Scenario: Handler is parse-call-render

- **WHEN** the handler runs
- **THEN** its body SHALL parse path/headers, call `PaymentAdminService::refund(...)`, and render based on the result; it SHALL NOT call `payment_repo`, `stripe_client`, `audit_service`, `integration_manager`, or `money_limiter` directly

