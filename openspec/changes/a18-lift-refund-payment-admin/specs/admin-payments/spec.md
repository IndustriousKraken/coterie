## ADDED Requirements

### Requirement: Refund handler lives in admin/payments.rs and routes through PaymentAdminService

The `admin_refund_payment` handler SHALL live in `src/web/portal/admin/payments.rs`, not in `src/web/portal/admin/members.rs`. The file location matches the URL path (`/portal/admin/payments/:id/refund`).

The handler SHALL parse the URL path parameter and the IP from headers, then call `PaymentAdminService::refund(current_user.id, payment_id, ip)`. The handler SHALL render `refund_result_html` based on the typed `Result<RefundOutcome, RefundError>` returned. Handler body SHALL be on the order of 25 lines; the orchestration chain is in the service, not here.

#### Scenario: Refund handler file location matches URL

- **WHEN** a contributor looks for the handler serving `POST /portal/admin/payments/:id/refund`
- **THEN** they SHALL find it in `src/web/portal/admin/payments.rs`, not in `members.rs`

#### Scenario: Handler is parse-call-render

- **WHEN** the handler runs
- **THEN** its body SHALL parse path/headers, call `PaymentAdminService::refund(...)`, and render based on the result; it SHALL NOT call `payment_repo`, `stripe_client`, `audit_service`, `integration_manager`, or `money_limiter` directly
