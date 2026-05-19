## Context

This change completes the admin-orchestration-lift series. After `lift-member-admin-orchestration` (members), `lift-event-admin-orchestration` (events), and `lift-announcement-admin-orchestration` (announcements), the remaining inline-orchestration handler is `admin_refund_payment` in `src/web/portal/admin/members.rs`.

The refund flow is the most stateful of the admin actions:
1. IP-based rate-limit check (refund attempts are money-moving and capped).
2. Load the payment row; reject if missing.
3. Validate status (already-refunded → no-op; not-completed → reject; waived → reject).
4. Atomic `claim_payment_for_refund` (Completed → Refunded conditional UPDATE). The claim is the lock; if it returns false, another admin won the race.
5. Branch on `payment_method`: Stripe → call Stripe's refund API with the local payment-id as idempotency-key (rollback the claim via `unclaim_refund` on Stripe error); Manual → no external call.
6. Build the human-readable detail string ("Refunded $X.YZ via Stripe (refund ri_…)").
7. Audit-log the refund.
8. Dispatch `IntegrationEvent::AdminAlert` so admins are notified of any refund.
9. Return the detail string for the HTML fragment.

All of that lives in the handler today. The pattern of moving it into a service is now well-established; the only design decisions specific to this lift are around the `RefundOutcome` shape and where the `MoneyLimiter` check lives.

## Goals / Non-Goals

**Goals:**
- `PaymentAdminService` owns the full refund chain.
- Handler shrinks to parse-and-render.
- Refund handler moves to `src/web/portal/admin/payments.rs` so its file location matches the URL it serves.
- Wire shape unchanged (URL, audit row, integration event, HTML fragment).

**Non-Goals:**
- New refund features (partial refunds, refund-of-refund, refund reasons). Today's behavior, just relocated.
- Touching the per-member payments-list view (`admin_member_payments` in members.rs). That handler reads payments for a member and renders the list; it doesn't mutate. Stays in members.rs (which is where the URL `/portal/admin/members/:id/payments` belongs).
- Changing how the integration alert renders. Same subject/body, same destination.
- Moving `record_manual_payment` (which already routes through `PaymentService::record_manual`). That handler is already correctly orchestrated and is logically a member-page action.

## Decisions

### D1. `PaymentAdminService::refund` takes the IP for rate-limit check

The rate-limit check is part of "what does a refund need to validate?" — same conceptual layer as "is the payment status Completed?". Putting it inside the service means a caller can't accidentally skip the limit by going around the handler.

The downside: services usually don't take IPs. The trade-off: this service owns money-moving actions, and money-moving actions are always per-IP rate-limited. Taking the IP as a parameter is consistent with that constraint, similar to how `PaymentService::record_manual` takes `actor_id` for audit provenance.

Alternative: leave the rate-limit check in the handler, just before calling the service. Less safe (a future caller — maybe a test — could skip the check), but simpler. Pick the safer shape (service owns the limit).

### D2. `RefundOutcome` carries everything the handler needs to render

```rust
pub struct RefundOutcome {
    pub amount_cents: i64,
    pub stripe_refund_id: Option<String>,
    pub detail: String,
    pub payment_method: PaymentMethod,
}
```

The handler renders `refund_result_html(true, &outcome.detail)`. It doesn't need to know how `detail` was built; the service constructs it the same way the handler does today.

The `payment_method` field is included for the integration-alert body (which references the method by Debug). Could also be derived by the handler if it re-fetched the payment, but cheaper to return it from the service.

### D3. Error shape: `Result<RefundOutcome, RefundError>`

A typed error enum lets the handler render the right user-facing message without parsing strings:

```rust
pub enum RefundError {
    RateLimited,
    InvalidPaymentId,
    PaymentNotFound,
    AlreadyRefunded,
    NotCompleted,
    WaivedNoRefund,
    StripeNotConfigured,
    NoStripeReferenceOnRecord,
    AnotherActorClaimedFirst,
    StripeApiError(String),  // carries the upstream message for logging
    InternalDatabaseError(AppError),
}
```

The handler's `match` on `RefundError` produces the appropriate `refund_result_html(false, message)` for each. The strings live in the handler (display concerns); the service stays display-agnostic.

Alternative: lump everything into `Result<RefundOutcome, AppError>` and let the handler match on `AppError` variants. Rejected — the existing `AppError` doesn't disambiguate "already refunded" from "Stripe rejected the refund," and adding new variants there pollutes the global error type for one handler.

### D4. Audit emission and integration dispatch in the service

Same pattern as the other admin services. Logged on failure inside the integration manager; service treats both as fire-and-forget. The audit row is written before the integration event (matches today's handler order).

### D5. Handler shrinks to ~25 lines

```rust
pub async fn admin_refund_payment(
    State(svc): State<Arc<PaymentAdminService>>,
    State(settings): State<Arc<Settings>>,
    Extension(current_user): Extension<CurrentUser>,
    headers: axum::http::HeaderMap,
    Path(payment_id): Path<String>,
) -> impl IntoResponse {
    let ip = crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for());
    let payment_uuid = match Uuid::parse_str(&payment_id) {
        Ok(id) => id,
        Err(_) => return refund_result_html(false, "Invalid payment ID"),
    };
    match svc.refund(current_user.member.id, payment_uuid, ip).await {
        Ok(outcome) => refund_result_html(true, &outcome.detail),
        Err(e) => refund_result_html(false, e.user_message()),
    }
}
```

`RefundError::user_message()` is a small `impl` returning `&'static str` for each variant. Keeps the handler short.

### D6. File location is `src/web/portal/admin/payments.rs`

The current route `/portal/admin/payments/:id/refund` is registered in the admin sub-router alongside `/portal/admin/members/*`. Moving the handler to `payments.rs` makes the URL-to-file mapping obvious. The router registration line in `src/web/portal/mod.rs` updates from `admin::members::admin_refund_payment` to `admin::payments::admin_refund_payment`.

If `admin/payments.rs` later gains other admin-payment actions (e.g., a payment-search page, a void action), they slot in naturally.

### D7. The `MoneyLimiter` lives on the service struct, not is a per-call parameter

```rust
pub struct PaymentAdminService {
    payment_repo: Arc<dyn PaymentRepository>,
    stripe_client: Option<Arc<StripeClient>>,
    audit_service: Arc<AuditService>,
    integration_manager: Arc<IntegrationManager>,
    money_limiter: MoneyLimiter,
}
```

The limiter is shared process-wide (via `RateLimiter`'s internal Arc); embedding it in the service is correct.

## Risks / Trade-offs

- **Risk**: a subtle behavior shift in the audit row or integration alert during the move. → Mitigation: line-by-line port; existing tests assert on the wire shape; new unit tests on `PaymentAdminService::refund` assert on the audit row's expected fields.
- **Risk**: the typed `RefundError` enum grows over time as new failure modes appear. → Acceptable: that's the point of a typed error. Each new variant is a compile-time signal that the handler's `user_message()` match needs to handle it.
- **Trade-off**: introducing a service for one method feels heavyweight. Trade is justified by (a) following the established pattern, (b) the method is genuinely complex (multi-step, atomic, branching on payment method), and (c) future refund-related admin actions slot in naturally.

## Migration Plan

Single PR.

1. Add `src/service/payment_admin_service.rs` with the struct, deps, and the `refund` method (full body ported from the existing handler).
2. Define `RefundOutcome` and `RefundError` in the same file. Add the `RefundError::user_message()` impl.
3. Register the service in `ServiceContext` + add `FromRef<AppState>` impl.
4. Create `src/web/portal/admin/payments.rs` with the new thin `admin_refund_payment` handler and the `refund_result_html` helper (moved from members.rs).
5. Add `pub mod payments;` to `src/web/portal/admin/mod.rs`.
6. Update the route registration in `src/web/portal/mod.rs` to point at the new module path.
7. Delete the old `admin_refund_payment` handler and `refund_result_html` helper from `src/web/portal/admin/members.rs`.
8. `cargo build --all-targets --features test-utils` — clean.
9. `cargo test --features test-utils` — full suite passes.
10. Add unit tests for `PaymentAdminService::refund` covering the seven scenarios in the proposal.
