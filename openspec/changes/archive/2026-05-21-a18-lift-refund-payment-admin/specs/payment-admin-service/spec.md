## ADDED Requirements

### Requirement: PaymentAdminService is the single entrypoint for admin-driven payment mutations

The system SHALL expose a `PaymentAdminService` at `src/service/payment_admin_service.rs` that owns the full side-effect chain (rate-limit enforcement, validation, atomic state transitions, external API calls, audit log, integration dispatch) for admin-driven payment mutations.

Today the only such mutation is refund. As future admin-payment actions are added (e.g., partial refund, void, manual status adjustment), they SHALL extend this service rather than re-implementing the chain inline in handlers.

#### Scenario: Handler calls the service, not the repo + collaborators

- **WHEN** an admin POSTs to `/portal/admin/payments/:id/refund`
- **THEN** the handler SHALL call `PaymentAdminService::refund(actor_id, payment_id, ip)` and render the response based on the returned `Result<RefundOutcome, RefundError>`; the handler SHALL NOT call `payment_repo.{claim_payment_for_refund, unclaim_refund, mark_refunded}`, `stripe_client.refund_payment`, `audit_service.log`, `integration_manager.handle_event`, or `money_limiter.check_and_record` directly

### Requirement: Refund owns the rate-limit check, not the handler

`PaymentAdminService::refund` SHALL take an `IpAddr` parameter and consult the `MoneyLimiter` instance held on the service struct. The handler SHALL NOT consult the limiter directly. This ensures a future caller (a test fixture, an alternative admin entry point) cannot accidentally skip the rate-limit by going around the handler.

#### Scenario: Rate-limit exceeded returns RefundError::RateLimited

- **WHEN** the same IP exceeds the money-limiter budget within the window and attempts a refund
- **THEN** the service SHALL return `Err(RefundError::RateLimited)` before any DB or Stripe activity; the handler renders the "too many refund attempts" message

### Requirement: Refund returns a typed RefundOutcome on success and typed RefundError on failure

The service SHALL return `Result<RefundOutcome, RefundError>` where `RefundOutcome` carries the data the handler needs to render a success fragment (`amount_cents`, `stripe_refund_id: Option<String>`, `detail: String`, `payment_method`). `RefundError` is an enum covering every distinct user-facing failure mode (`RateLimited`, `PaymentNotFound`, `AlreadyRefunded`, `NotCompleted`, `WaivedNoRefund`, `StripeNotConfigured`, `NoStripeReferenceOnRecord`, `AnotherActorClaimedFirst`, `StripeApiError(String)`, `InternalDatabaseError(AppError)`, etc.).

`RefundError::user_message()` SHALL return a `&'static str` for each variant. The handler renders that message in the failure fragment.

#### Scenario: Stripe refund success returns outcome with refund id

- **WHEN** a Stripe-method payment is successfully refunded
- **THEN** the returned `RefundOutcome.stripe_refund_id` is `Some(ri_…)` and `detail` reads "Refunded $X.YZ via Stripe (refund ri_…)"

#### Scenario: Manual refund success has no Stripe id

- **WHEN** a Manual-method payment is refunded
- **THEN** `stripe_refund_id` is `None` and `detail` reads "Marked $X.YZ manual payment as Refunded (no API call — refund the cash/check yourself)"

#### Scenario: Stripe API error rolls back the claim

- **WHEN** the claim succeeds but Stripe's refund API returns an error
- **THEN** the service SHALL call `payment_repo.unclaim_refund(payment_id)` to revert the Completed→Refunded flip, and return `Err(RefundError::StripeApiError(message))`; a subsequent retry can re-claim the row

### Requirement: Service inherits existing failure semantics

`PaymentAdminService::refund` SHALL preserve the existing semantics:

- Audit-log insert failure: logged via `tracing`, swallowed.
- Integration dispatch failure: per-integration failures logged inside `IntegrationManager`; the call returns success.
- Repository failure: propagated as `RefundError::InternalDatabaseError`.

#### Scenario: Integration dispatch failure does not roll back the refund

- **WHEN** the refund succeeds at the repo + Stripe level but the AdminAlert dispatch fails
- **THEN** the service SHALL return `Ok(outcome)`; the integration failure SHALL be logged inside the integration layer
