# Design notes — signup-verify-email-before-checkout

Guidance, not contract. The invariant is the whole point: **a Stripe Checkout
session (and the customer it needs) is created only for an email-verified
member.** Every checkout-creation site must honor it.

## Flow

Before: `POST /public/signup` (payment, fee>0) → create Pending member → queue
verification email → **create Stripe customer + Checkout** → return checkout URL.

After:
1. `POST /public/signup` (payment, fee>0) → create Pending member → queue
   verification email → return "check your email to continue" (**no** Stripe, **no**
   URL).
2. Member clicks the verification link → `verify_handler` marks verified → if
   Pending + unpaid + payment-mode + fee>0, **create the Checkout session now** and
   present/redirect to it.
3. Member pays → `checkout.session.completed` webhook → activate (unchanged).

## Checkout-creation sites (all must be gated on verified)

- **Initial:** moves from the signup handler to `verify_handler`.
- **Retry** ("Abandoned signup checkout is retryable"): already mints a session
  for a Pending member whose password verifies — add the `email_verified`
  precondition; an unverified retry re-queues the verification email instead. This
  matters because the bot set (and therefore knows) its own password, so the retry
  path would otherwise be an unverified back door to Stripe.

Refactor `create_signup_checkout` (currently called from the signup handler) into
a helper callable from both `verify_handler` and the retry path, so the session
contract + auto-renew (`membership.signup_auto_renew`) handling stays identical
and in one place.

## Why this kills card-testing

The attack needs one unauthenticated hop to Stripe. Requiring a clicked
verification link means the attacker must control a real, monitored inbox per
attempt — which defeats harvested-address / Gmail-dot-trick bots and does not
scale. It composes with Turnstile (separate change): Turnstile stops the junk
Pending row from being created at all; this stops any signup — junk or not — from
reaching Stripe unverified.

## UX / edge cases

- The verify result page changes for the paid-signup case from "an administrator
  will review your account" to continuing into payment (redirect or a prominent
  "Continue to payment" button).
- A member who verifies but abandons the Stripe page can re-enter via the retry
  path (now allowed because they are verified).
- Verification tokens already expire; an expired link → the member re-signs up
  (retry path) which re-queues verification. No change needed there.
- The marketing join form's success copy changes from "redirecting to payment" to
  "check your email"; that's a companion tweak in the `theneontemple.com` repo,
  not governed by this spec.
