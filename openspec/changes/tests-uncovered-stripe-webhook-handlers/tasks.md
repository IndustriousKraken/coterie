## 1. Add cfg-gated test seams for the three handlers

- [ ] 1.1 In `src/payments/webhook_dispatcher/mod.rs`, under the existing
  `#[cfg(any(test, feature = "test-utils"))] impl WebhookDispatcher` block,
  add `pub async fn dispatch_failed_payment(&self, stripe_payment_id: String) -> Result<()>`
  forwarding to `self.handle_failed_payment(stripe_payment_id)`.
- [ ] 1.2 Add `pub async fn dispatch_expired_session(&self, session: CheckoutSession) -> Result<()>`
  forwarding to `self.handle_expired_session(session)`.
- [ ] 1.3 Add `pub async fn dispatch_subscription_updated(&self, subscription: stripe::Subscription) -> Result<()>`
  forwarding to `self.handle_subscription_updated(subscription)`.

## 2. Tests for handle_failed_payment

- [ ] 2.1 `failed_payment_flips_matching_payment_to_failed` — seed a
  `Pending` payment whose `external_id` is `StripeRef::PaymentIntent(pi_id)`;
  call `dispatch.dispatch_failed_payment(pi_id.to_string())`; assert the
  payment row's `status` is now `PaymentStatus::Failed`.
- [ ] 2.2 `failed_payment_for_unknown_id_is_noop` — with no matching payment,
  call `dispatch_failed_payment("pi_does_not_exist".into())`; assert it
  returns `Ok(())`, no panic, and no payment row changes status.

## 3. Tests for handle_expired_session

- [ ] 3.1 `expired_session_flips_pending_payment_to_failed` — seed a
  `Pending` payment whose `external_id` is `StripeRef::CheckoutSession(cs_id)`;
  build a `CheckoutSession` with `id = cs_id` (reuse the JSON-deserialization
  pattern already in `tests/stripe_webhook_test.rs`); call
  `dispatch.dispatch_expired_session(session)`; assert the payment row's
  `status` is now `Failed`.
- [ ] 3.2 `expired_session_for_unknown_session_is_noop` — dispatch a
  `CheckoutSession` whose id matches no payment; assert `Ok(())`, no panic,
  and no payment row changes status.

## 4. Tests for handle_subscription_updated

- [ ] 4.1 `subscription_updated_refreshes_stored_subscription_id` — seed a
  member with a known `stripe_customer_id` and an old
  `stripe_subscription_id`; build a `stripe::Subscription` with that customer
  and a NEW subscription id; call
  `dispatch.dispatch_subscription_updated(subscription)`; assert the member
  row's stored subscription id now equals the new id.
- [ ] 4.2 `subscription_updated_for_unknown_customer_is_noop` — dispatch a
  `Subscription` whose customer maps to no member; assert `Ok(())`, no panic,
  and no member row is mutated.
