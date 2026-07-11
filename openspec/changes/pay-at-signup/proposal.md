# pay-at-signup

## Why

The current signup funnel is approval-gated: `POST /public/signup` creates a
`Pending` member, verification email goes out, and the member stays Pending
until an admin manually activates them (`web/portal/admin/members/status.rs`);
dues are paid later inside the portal. For orgs that want low-friction,
high-automation onboarding — visitor signs up, pays, and is a member, with
downstream automation (e.g. Discord invite once that integration is enabled)
triggered by activation — the manual-approval step is the bottleneck, and
there is no way to collect payment at signup.

All the machinery already exists: the public donate flow proves out
unauthenticated Stripe Checkout; `StripeClient::create_membership_checkout_session`
(`payments/stripe_client.rs:62`) stamps the `payment_type=membership` +
`membership_type_slug` metadata the webhook already honors to record the
payment and extend dues. What's missing is: a signup mode where the signup
response hands back a checkout URL, an activation rule (completed membership
payment flips Pending → Active), and the money-moving gates on signup when it
initiates payment.

Coterie stays org-agnostic: approval-gated signup remains the default;
pay-at-signup is an org setting.

## What Changes

- New org setting `membership.signup_mode`: `approval` (default — current
  behavior, unchanged) or `payment`.
- In `payment` mode, `POST /public/signup` with a membership type whose fee
  is > 0: create the member (Pending) and a Stripe Checkout session for that
  type's dues (reusing `create_membership_checkout_session`, so the existing
  webhook path records the payment and extends dues with no new webhook
  code), and return the checkout URL to the caller for redirect. A type with
  fee 0 behaves as in approval mode (member stays Pending; no checkout).
- Activation on payment: a completed membership payment for a `Pending`
  member SHALL transition them to `Active` (extending the existing
  Expired→Active revival in `extend_dues_for_payment_atomic`,
  `payment_repository.rs:533`) and dispatch the member-activated integration
  event — the seam for the future Discord-invite automation.
- Gates: in payment mode signup initiates payment, so it inherits the
  money-moving gate set per the public-donate capability: `money_limiter`
  FIRST, then bot challenge, plus the CORS allowlist. Approval mode keeps
  the current gates (no money limiter).
- Abandoned-checkout retry: in payment mode, re-POSTing signup with the
  email of an existing Pending member who has no completed payment, with a
  password that verifies against that member, returns a fresh checkout URL
  instead of a duplicate-email error — so an abandoned checkout does not
  strand the signup.
- Email verification is unchanged and independent: the verification email is
  still sent at signup; login remains gated on member status
  (`api/handlers/auth.rs:91`), and verification stamps `email_verified_at`
  as today.

## Impact

- Spec: `public-signup` — 2 MODIFIED requirements (signup gating text;
  Pending-login/activation-source requirement) and 4 ADDED requirements
  (signup-mode setting; payment-mode checkout; activation on completed
  payment; abandoned-checkout retry).
- Code: signup handler branch (`api/handlers/public.rs`), org-setting seed
  migration, `extend_dues_for_payment_atomic` revival extension +
  integration-event dispatch, retry path.
- Tests: approval-mode regression (default unchanged), payment-mode
  checkout via `FakeStripeGateway`, webhook activates Pending, gate order,
  fee-0, retry (right and wrong password).
- The marketing-site form changes (fetch types, redirect to checkout URL)
  live in the site repo, not here.
