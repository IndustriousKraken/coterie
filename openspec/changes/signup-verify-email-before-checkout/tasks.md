# Tasks

The invariant to hold everywhere: a Stripe Checkout session / customer is created
ONLY for an email-verified member.

## 1. Signup handler — stop reaching Stripe at signup

- [ ] 1.1 `src/api/handlers/public.rs` signup: in the payment-mode branch, create
  the Pending member and queue the verification email as today, but REMOVE the
  Stripe customer + Checkout session creation. Return a success response that
  instructs the caller to verify their email and carries NO checkout URL.
- [ ] 1.2 Factor the existing `create_signup_checkout` (session contract +
  `membership.signup_auto_renew` handling) into a helper callable from the verify
  handler and the retry path — do not duplicate the session logic.

## 2. Verify handler — initiate checkout on verification

- [ ] 2.1 `src/web/templates/verify.rs`: after `mark_email_verified`, if the
  member is `Pending`, has no completed membership payment, signup mode is
  `payment`, and their type fee > 0, call the checkout helper and direct the
  member to the session (redirect or a "Continue to payment" result). Otherwise
  keep the existing "verified; awaiting review" result. Handle a checkout-creation
  failure softly (verified stands; show a retry path / message).

## 3. Retry path — require verification

- [ ] 3.1 In the abandoned-checkout retry (`retry_pending_checkout`), add the
  `email_verified` precondition: only mint/return a session for a verified member.
  For an unverified Pending member with the correct password, re-queue the
  verification email and return no checkout URL. Wrong-password / paid / non-Pending
  outcomes are unchanged (existing duplicate handling).

## 4. Tests

- [ ] 4.1 Payment-mode signup (fee>0) → response has no checkout URL and NO Stripe
  customer/session is created.
- [ ] 4.2 Verifying a Pending, unpaid, payment-mode, fee>0 member → checkout
  session initiated and presented.
- [ ] 4.3 Approval-mode / fee-0 verification → verified + awaiting-review, no
  checkout.
- [ ] 4.4 Unverified retry (correct password) → verification re-queued, no session.
- [ ] 4.5 Verified retry → resumes open session or refreshes an expired one
  (existing retry behavior preserved).
- [ ] 4.6 Completed-payment webhook still activates the Pending member (unchanged).

## 5. Verify

- [ ] 5.1 `openspec validate signup-verify-email-before-checkout --strict` passes.
- [ ] 5.2 `cargo test` (signup / webhook / verify suites) green; `cargo clippy` clean.

## 6. Companion (marketing repo, not this spec)

- [ ] 6.1 `theneontemple.com` join-form success copy: "check your email to
  continue" instead of "redirecting to payment".
