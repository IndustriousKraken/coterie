## Why

`admin_refund_payment` in `src/web/portal/admin/members.rs` (~140 lines, roughly lines 628–770 as of this writing) is the last admin mutation handler that still does inline orchestration: rate-limit check, payment lookup, status validation, atomic claim-then-call-Stripe, audit log, integration dispatch. It was deliberately scoped out of `lift-member-admin-orchestration` at the time — the rationale was "it operates on payments, not members, so it doesn't fit MemberService."

That rationale stands; the handler doesn't belong in `MemberService`. But it also doesn't belong in the members admin handler file, and it doesn't belong inline anywhere. The pattern established by `MemberService`, `EventAdminService`, and `AnnouncementAdminService` is the right home: a `PaymentAdminService` that owns the validate → claim → call-Stripe → audit → integration chain.

An architectural reviewer flagged that `members.rs` has grown beyond its coherent identity. Moving refund out is half of the answer (the other half — extracting bulk CSV handlers — is a separate change). After this change lands, every admin mutation in the codebase goes through a per-domain service, with zero exceptions.

## What Changes

- **Add `PaymentAdminService`** at `src/service/payment_admin_service.rs`. Methods (v1, scoped to what `admin_refund_payment` does today):
  - `refund(actor_id: Uuid, payment_id: Uuid, ip: IpAddr) -> Result<RefundOutcome>` — the full chain that currently lives in the handler.
- **`RefundOutcome` typed result**: small struct carrying `amount_cents`, optional `stripe_refund_id`, and a `detail: String` for the human-readable summary that the response fragment renders. The handler does the HTML rendering; the service returns the typed outcome.
- **`PaymentAdminService` deps**: `Arc<dyn PaymentRepository>`, `Option<Arc<StripeClient>>`, `Arc<AuditService>`, `Arc<IntegrationManager>`, `MoneyLimiter`. Same DI-via-AppState pattern as the other admin services. The `MoneyLimiter` lives on the service so the limit check is part of the service's contract, not the handler's responsibility.
- **Plumb `payment_admin_service` through `ServiceContext` + `AppState` + `FromRef<AppState>` impl**.
- **Replace `admin_refund_payment` handler body** with parse-id → call-service → render. The handler shrinks from ~140 lines to ~25 lines (parameter extraction + the `match` on the service result + HTML rendering).
- **Move the handler to a more honest location**: `admin_refund_payment` currently lives in `src/web/portal/admin/members.rs`. Move it to a new `src/web/portal/admin/payments.rs` module so the file's identity matches the URL path (`/portal/admin/payments/:id/refund`). The route registration in `src/web/portal/mod.rs` updates to point at the new module.
- **Keep `refund_result_html` helper** alongside the handler in the new location.
- **Out of scope**: the rate-limiter newtype (`MoneyLimiter`) is already established as the way to inject limiters. No changes to its shape.

## Capabilities

### New Capabilities
- `payment-admin-service`: single entrypoint for admin-driven payment mutations. Today: refund. Future-extensible: any admin payment-level action (manual refund-of-refund, partial refund, etc.) would land here.

### Modified Capabilities
- `admin-members`: the spec's "Member-page payment actions live on the per-member page" requirement updates — the refund handler still POSTs to the same URL, but the handler lives in `admin/payments.rs` rather than `admin/members.rs`. The scenario about routing through `PaymentService` becomes routing through `PaymentAdminService`.
- `admin-payments`: extends to cover the refund flow's service-locus contract.
- `audit-logging`: refund operations join the service-locus column.

## Impact

- **Code**:
  - New ~200-line `src/service/payment_admin_service.rs`.
  - New ~30-line `src/web/portal/admin/payments.rs` (just the refund handler + `refund_result_html` helper).
  - `src/web/portal/admin/members.rs` shrinks by ~150 lines (refund handler + helper move out).
  - `src/web/portal/admin/mod.rs` gains a `pub mod payments;` line.
  - `src/web/portal/mod.rs` route registration updates: `.route("/payments/:id/refund", post(admin::payments::admin_refund_payment))`.
  - `src/api/state.rs` gains a FromRef impl for `Arc<PaymentAdminService>`.
- **Wire shape**: zero change. Same URL, same form, same audit row content, same integration alert, same response fragment.
- **Tests**: existing handler-level tests pass unchanged. Add unit tests for the service covering: happy-path Stripe refund, happy-path manual refund, double-click claim race, already-refunded short-circuit, waived-payment rejection, Stripe-error unclaim-and-fail, rate-limit rejection.
- **Risk**: low. Pattern is well-established by the three prior orchestration lifts.
- **Dependency**: none. Independent of every other queued change.
