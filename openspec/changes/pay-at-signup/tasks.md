# Tasks

## 1. Setting

- [x] 1.1 Migration: seed `membership.signup_mode` = `approval` in
  `app_settings` (category `membership`, so the generic admin settings page
  renders it). Add a `signup_mode()` accessor on `SettingsService` that
  defaults to approval on missing/unknown values. (The dead 001-era
  `membership.auto_approve` / `membership.require_payment_for_activation`
  rows this supersedes were already dropped by migration 036.)

## 2. Signup handler (payment mode)

- [x] 2.1 In `api/handlers/public.rs::signup`, when
  `signup_mode == payment` and the resolved membership type has
  `fee_cents > 0`: after creating the Pending member and queueing the
  verification email, call
  `StripeClient::create_membership_checkout_session` for that member/type
  and include the checkout URL in the 200 response body
  (`{ "checkout_url": ... }`); approval mode and fee-0 types return the
  current response shape (no URL).
- [x] 2.2 Gate order in payment mode: `money_limiter` check FIRST, then bot
  challenge, then validation — mirroring the donate handler. Approval mode
  gate order unchanged.
- [x] 2.3 Retry path: on duplicate email where the existing member is
  Pending with no completed membership payment AND the supplied password
  verifies against their hash, return a fresh checkout session instead of
  the duplicate error. Wrong password → exactly the current duplicate
  outcome.

## 3. Activation on completed payment

- [x] 3.1 Extend `extend_dues_for_payment_atomic`
  (`repository/payment_repository.rs`) revival semantics: a completed
  membership payment for a `Pending` member sets status `Active` (same
  transactional claim as the Expired→Active flip).
- [x] 3.2 Dispatch the member-activated integration event on that
  transition (same event admin activation dispatches), so Discord/UniFi
  automation sees payment-activated members identically.

## 4. Tests

- [x] 4.1 Regression: default (approval) mode — signup response shape and
  funnel unchanged; no checkout session created (FakeStripeGateway records
  zero calls).
- [x] 4.2 Payment mode: signup creates Pending member + returns checkout
  URL (FakeStripeGateway); metadata carries member_id +
  payment_type=membership + membership_type_slug.
- [x] 4.3 Webhook completion for that session activates the Pending member
  and extends dues; integration event dispatched.
- [x] 4.4 Payment mode + fee-0 type: no checkout, member stays Pending.
- [x] 4.5 Gate order: IP over money-limiter budget gets 429 before the
  bot-challenge verifier is called (payment mode only).
- [x] 4.6 Retry: duplicate email + correct password + no completed payment
  → fresh checkout URL; duplicate email + wrong password → duplicate error;
  duplicate email + completed payment → duplicate error.

## 5. Retry reuses the open session (no duplicate pending rows)

- [x] 5.1 Extend `RetrievedCheckoutSession` with `is_open` + `url`
  (real gateway maps Stripe's session status/url; fake gateway defaults
  updated).
- [x] 5.2 Retry path: retrieve the member's most recent pending
  checkout session; if open, return its URL (no new session, no new
  payment row); otherwise `fail_pending_payment` the stale row and mint
  a fresh session.
- [x] 5.3 Tests: open-session reuse returns the same URL with no new
  payment row; non-open session marks the old row Failed and mints new.

## 6. Auto-renew enrollment by default

- [x] 6.1 Migration: seed `membership.signup_auto_renew` = `true`
  (boolean, membership category) + `signup_auto_renew()` accessor.
- [x] 6.2 `CreateCheckoutInput` gains `customer_id` +
  `save_card_for_offsession` (real gateway sets `customer` and
  `payment_intent_data.setup_future_usage=off_session`);
  `create_membership_checkout_session` takes customer/save-card params
  and stamps `save_card=true` metadata (portal call site unchanged:
  None/false).
- [x] 6.3 Signup handler: when the setting is on,
  `get_or_create_customer` for the new member and create the session
  with customer + save-card.
- [x] 6.4 `AutoRenew::enroll_after_signup_payment(member, slug,
  customer)`: list the customer's payment methods, save unseen cards
  (fingerprint-deduped, default when none), then `enable_auto_renew`.
  Called from the checkout-completed webhook when the session metadata
  carries `save_card=true`; soft-fails (logged, never fails the
  webhook).
- [x] 6.5 Tests: session creation carries customer + metadata when the
  setting is on and not when off; webhook completion enrolls (saved
  card + coterie_managed + scheduled payment); enrollment failure
  leaves the member Active and the webhook Ok.
