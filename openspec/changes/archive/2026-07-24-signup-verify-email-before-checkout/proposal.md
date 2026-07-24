# signup-verify-email-before-checkout

## Why

The public payment-mode signup funnel is being abused for **card-testing**: bots
POST `/public/signup` with harvested emails and random usernames, which
immediately creates a Stripe customer + Checkout session — letting attackers run
stolen cards through the org's live Stripe. A recent live incident produced 62
bot accounts and a wall of `Failed` charges (the kind of decline rate that gets a
Stripe account flagged).

The root enabler is that **an unauthenticated request reaches Stripe in one hop**.
This change gates the Stripe surface behind a **verified email**: payment-mode
signup creates the Pending member and sends the verification email, but does NOT
create a Stripe customer or Checkout session until the member clicks the
verification link. A bot using a fake/unmonitored inbox can never verify, so it
never reaches checkout — which, per the operator, "would probably get rid of all
bots on its own." It also composes with the (separate) Turnstile bot-challenge.

This changes the signup funnel's behavior, so it is a **change**.

## What Changes

- **Payment-mode signup no longer initiates checkout.** `POST /public/signup`
  (payment mode, fee > 0) creates the Pending member and queues the verification
  email, but creates **no Stripe customer and no Checkout session**, and returns
  **no checkout URL** — the response tells the caller to verify their email to
  continue to payment.
- **Verification initiates checkout.** When a Pending, unpaid, payment-mode
  member with a fee > 0 type verifies their email, the verify handler initiates
  the Stripe Checkout session (same session contract + auto-renew handling as
  before) and directs them to it. Approval mode, fee-0 types, and
  already-verified/paid members keep their prior verification behavior.
- **The retry path requires verification too.** The abandoned-checkout retry only
  mints a session for a member whose email is verified; an unverified retry
  re-queues the verification email instead (so a bot that knows its own password
  can't bypass the gate via retry).
- **Invariant:** a Stripe Checkout session is only ever created for a member whose
  email is verified.
- Webhook activation on completed payment is unchanged.

## Impact

- **Spec:** `public-signup` — 2 MODIFIED requirements ("Payment-mode signup
  initiates Stripe checkout", "Abandoned signup checkout is retryable") + 1 ADDED
  ("Email verification initiates payment-mode checkout").
- **Code:** `src/api/handlers/public.rs` (signup: drop checkout/customer creation
  in the payment branch; response message change), `src/web/templates/verify.rs`
  (on verifying a payment-mode unpaid Pending member, create the checkout and
  redirect/present it — factor the existing `create_signup_checkout` so both the
  verify handler and the retry path call it), retry path gains the verified
  precondition.
- **Marketing funnel:** the join form's success UX changes from "redirecting to
  payment" to "check your email to continue" (companion tweak in the
  `theneontemple.com` repo; not governed by this spec).
- **Tests:** payment-mode signup returns no checkout URL and creates no Stripe
  customer; verifying a paid signup opens checkout; approval/fee-0 verification
  unchanged; unverified retry re-queues verification and mints no session;
  verified retry still resumes/refreshes checkout.
- **Trade-off:** legit paying members click one email link before paying — the
  intended friction, and standard practice.
