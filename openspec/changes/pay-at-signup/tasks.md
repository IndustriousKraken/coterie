# Tasks

## 1. Setting

- [ ] 1.1 Migration: seed `membership.signup_mode` = `approval` in
  `app_settings` (category `membership`, so the generic admin settings page
  renders it). Add a `signup_mode()` accessor on `SettingsService` that
  defaults to approval on missing/unknown values.

## 2. Signup handler (payment mode)

- [ ] 2.1 In `api/handlers/public.rs::signup`, when
  `signup_mode == payment` and the resolved membership type has
  `fee_cents > 0`: after creating the Pending member and queueing the
  verification email, call
  `StripeClient::create_membership_checkout_session` for that member/type
  and include the checkout URL in the 200 response body
  (`{ "checkout_url": ... }`); approval mode and fee-0 types return the
  current response shape (no URL).
- [ ] 2.2 Gate order in payment mode: `money_limiter` check FIRST, then bot
  challenge, then validation — mirroring the donate handler. Approval mode
  gate order unchanged.
- [ ] 2.3 Retry path: on duplicate email where the existing member is
  Pending with no completed membership payment AND the supplied password
  verifies against their hash, return a fresh checkout session instead of
  the duplicate error. Wrong password → exactly the current duplicate
  outcome.

## 3. Activation on completed payment

- [ ] 3.1 Extend `extend_dues_for_payment_atomic`
  (`repository/payment_repository.rs`) revival semantics: a completed
  membership payment for a `Pending` member sets status `Active` (same
  transactional claim as the Expired→Active flip).
- [ ] 3.2 Dispatch the member-activated integration event on that
  transition (same event admin activation dispatches), so Discord/UniFi
  automation sees payment-activated members identically.

## 4. Tests

- [ ] 4.1 Regression: default (approval) mode — signup response shape and
  funnel unchanged; no checkout session created (FakeStripeGateway records
  zero calls).
- [ ] 4.2 Payment mode: signup creates Pending member + returns checkout
  URL (FakeStripeGateway); metadata carries member_id +
  payment_type=membership + membership_type_slug.
- [ ] 4.3 Webhook completion for that session activates the Pending member
  and extends dues; integration event dispatched.
- [ ] 4.4 Payment mode + fee-0 type: no checkout, member stays Pending.
- [ ] 4.5 Gate order: IP over money-limiter budget gets 429 before the
  bot-challenge verifier is called (payment mode only).
- [ ] 4.6 Retry: duplicate email + correct password + no completed payment
  → fresh checkout URL; duplicate email + wrong password → duplicate error;
  duplicate email + completed payment → duplicate error.
