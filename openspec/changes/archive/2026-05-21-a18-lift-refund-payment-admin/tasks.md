## 1. Service skeleton

- [x] 1.1 Create `src/service/payment_admin_service.rs` with the struct `PaymentAdminService` holding `payment_repo: Arc<dyn PaymentRepository>`, `stripe_client: Option<Arc<StripeClient>>`, `audit_service: Arc<AuditService>`, `integration_manager: Arc<IntegrationManager>`, `money_limiter: MoneyLimiter`. Add `pub fn new(...)` constructor.
- [x] 1.2 Define `RefundOutcome` and `RefundError` enum + `impl RefundError { pub fn user_message(&self) -> &'static str { … } }` in the same file. Variants per design.md D3.
- [x] 1.3 Register `pub mod payment_admin_service;` in `src/service/mod.rs`.
- [x] 1.4 Add `pub payment_admin_service: Arc<PaymentAdminService>` to `ServiceContext` and construct it inside `ServiceContext::new`.
- [x] 1.5 Add `impl FromRef<AppState> for Arc<PaymentAdminService>` in `src/api/state.rs`.
- [x] 1.6 `cargo build` — clean. The empty service is plumbed but unused.

## 2. Implement `refund`

- [x] 2.1 Add `pub async fn refund(&self, actor_id: Uuid, payment_id: Uuid, ip: IpAddr) -> Result<RefundOutcome, RefundError>` to `PaymentAdminService`.
- [x] 2.2 Port the body from the existing `admin_refund_payment` handler in `src/web/portal/admin/members.rs`. Steps in order:
  1. `if !self.money_limiter.0.check_and_record(ip) { return Err(RefundError::RateLimited); }`
  2. `let payment = self.payment_repo.find_by_id(payment_id).await.map_err(RefundError::InternalDatabaseError)?.ok_or(RefundError::PaymentNotFound)?;`
  3. Status validation: already-refunded → `Err(RefundError::AlreadyRefunded)`; not-completed → `Err(RefundError::NotCompleted)`; waived → `Err(RefundError::WaivedNoRefund)`.
  4. Atomic claim: `self.payment_repo.claim_payment_for_refund(payment.id)` — false → `Err(RefundError::AnotherActorClaimedFirst)`.
  5. Branch on `payment.payment_method`:
     - `Stripe`: validate `external_id` is non-empty (else unclaim + `Err(RefundError::NoStripeReferenceOnRecord)`); validate `stripe_client.is_some()` (else unclaim + `Err(RefundError::StripeNotConfigured)`); call `stripe_client.refund_payment(stripe_ref, &payment.id.to_string()).await` — on error: unclaim + `Err(RefundError::StripeApiError(message))`.
     - `Manual`: no external call; `stripe_refund_id = None`.
     - `Waived`: unreachable (validated above).
  6. Build the `detail` string using the same `match (&payment.payment_method, &stripe_refund_id)` shape as today.
  7. `self.audit_service.log(Some(actor_id), "refund_payment", "payment", &payment_id.to_string(), None, Some(&detail), None).await;`
  8. `self.integration_manager.handle_event(IntegrationEvent::AdminAlert { subject: …, body: … }).await;` (build subject/body the same way the handler does today).
  9. Return `Ok(RefundOutcome { amount_cents, stripe_refund_id, detail, payment_method })`.

## 3. New handler module

- [x] 3.1 Create `src/web/portal/admin/payments.rs` with `pub async fn admin_refund_payment(...)` and the `fn refund_result_html(ok: bool, detail: &str) -> Html<String>` helper (move both from `members.rs`).
- [x] 3.2 The new handler signature uses granular extractors: `State<Arc<PaymentAdminService>>`, `State<Arc<Settings>>`, `Extension<CurrentUser>`, `headers: HeaderMap`, `Path<String>`. Drops `State<Arc<PaymentRepository>>`, `State<Option<Arc<StripeClient>>>`, `State<Arc<AuditService>>`, `State<Arc<IntegrationManager>>`, `State<MoneyLimiter>` — all moved into the service.
- [x] 3.3 Body: parse IP from headers (via `crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for())`); parse the path UUID (return `refund_result_html(false, "Invalid payment ID")` on parse failure); call `svc.refund(current_user.member.id, payment_uuid, ip).await`; match the result and render.
- [x] 3.4 Add `pub mod payments;` to `src/web/portal/admin/mod.rs`.

## 4. Route registration

- [x] 4.1 In `src/web/portal/mod.rs`, find the line registering the refund route. Today it likely reads `.route("/payments/:id/refund", post(admin::members::admin_refund_payment))`. Update to `.route("/payments/:id/refund", post(admin::payments::admin_refund_payment))`.

## 5. Remove old handler from members.rs

- [x] 5.1 Delete the `admin_refund_payment` function (the long one — currently ~lines 628–770 in `members.rs`).
- [x] 5.2 Delete the `refund_result_html` helper from `members.rs` (it's moved to `payments.rs`).
- [x] 5.3 Sweep unused imports in `members.rs`. Likely removable: `StripeClient`, `IntegrationManager` direct usage (if no other handler in the file uses them after refund moves out), `MoneyLimiter`, `PaymentRepository` (if nothing else in members.rs uses it).
- [x] 5.4 Confirm `cargo build` is clean.

## 6. Test pass

- [x] 6.1 Run `cargo test --features test-utils`. The existing handler-level tests for refund (if any in `tests/`) assert on HTTP responses; they continue to pass.
- [x] 6.2 Add unit tests in `payment_admin_service.rs::tests`:
  - happy-path Stripe refund (asserts: claim → stripe called → audit row → integration event → success outcome).
  - happy-path manual refund (asserts: claim → no stripe call → audit row with manual message → integration event → success outcome with `stripe_refund_id = None`).
  - already-refunded short-circuit (asserts: returns `Err(AlreadyRefunded)`, no claim, no stripe call, no audit, no integration).
  - not-completed rejection (returns `Err(NotCompleted)`, no side effects).
  - waived rejection (returns `Err(WaivedNoRefund)`, no side effects).
  - double-click claim race: two concurrent calls — exactly one returns `Ok`, the other returns `Err(AnotherActorClaimedFirst)`.
  - stripe-api error: claim succeeds, Stripe rejects, expect `Err(StripeApiError)` AND that `unclaim_refund` was called.
  - rate-limit rejection: pre-populate the limiter to be over budget, expect `Err(RateLimited)` with no DB or Stripe activity.

## 7. Verify

- [x] 7.1 `cargo build --all-targets --features test-utils` — clean.
- [x] 7.2 `cargo test --features test-utils` — all green including new unit tests.
- [x] 7.3 Eyeball: `wc -l src/web/portal/admin/members.rs` should drop by ~150 lines.
- [x] 7.4 Eyeball: `wc -l src/web/portal/admin/payments.rs` should be ~30 lines (just the handler + helper).
