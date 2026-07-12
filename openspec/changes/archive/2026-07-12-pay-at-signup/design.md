# pay-at-signup — design notes

## Signup mode is a setting, defaulting to approval

Coterie serves orgs that vet members (the current flow) and orgs that want
frictionless paid onboarding. `membership.signup_mode` (`approval` |
`payment`) captures that; `approval` is the default so existing deployments
are untouched. The key lives in the `membership` category, which the generic
admin `/settings` page already renders — no new admin UI needed, just a
seed migration.

## Reuse the portal checkout contract wholesale

`create_membership_checkout_session` already stamps `member_id`,
`payment_type=membership`, `membership_type_slug` into session metadata, and
the `checkout.session.completed` webhook already records the payment and
extends dues off exactly those keys (`webhook_dispatcher/checkout.rs`). The
signup handler calls the same method for the just-created Pending member.
No new webhook branch, no new metadata contract. Success/cancel URLs come
from the existing `stripe.success_url` / `stripe.cancel_url` settings — the
operator points them at the marketing site's welcome/cancel pages. (A
per-request return path restricted to CORS-allowlisted origins is a
possible later refinement; not in scope.)

## Activation: completed membership payment flips Pending → Active

`extend_dues_for_payment_atomic` already has revival semantics for
Expired→Active. Extending the same rule to Pending is deliberate and
universal (not gated on signup_mode):

- In payment mode it is the whole point — payment is the approval.
- In approval mode, a Pending member cannot log in, cannot reach portal
  checkout, and thus cannot produce a membership payment without an admin's
  involvement (e.g. admin records a manual dues payment). If an admin
  records dues for a Pending member, activating them is the intended
  outcome — dues-on-record with a dormant account is a bug, not a feature.

Activation through this path SHALL dispatch the same integration event as
admin activation, so integrations (Discord role/invite, UniFi later) behave
identically regardless of how a member became Active.

Login/verification is deliberately untouched: login gates on status only
(`api/handlers/auth.rs:91-92`), verification stamps `email_verified_at`. A
paid-but-unverified member can log in — same trust level as an
admin-activated-but-unverified member today.

## Abandoned checkout must not strand the member

In payment mode the failure path is: member row created (Pending), checkout
never completed. That member cannot log in (Pending) and cannot re-signup
(duplicate email). The retry rule: signup POST hitting an existing Pending
member with no completed membership payment, where the supplied password
verifies against that member's hash, returns a fresh checkout session
rather than a duplicate error. The password check keeps this from becoming
an oracle: a wrong password gets the same duplicate-email outcome as today,
so the endpoint leaks nothing beyond what duplicate detection already does.

## Gate order

Payment mode makes signup a money-moving endpoint; per the public-donate
capability's standing requirement, all three gates apply and
`money_limiter` runs FIRST (a bursting IP must not burn bot-challenge
quota). In approval mode the existing gate set (bot challenge, CORS, no
money limiter) is preserved — the canonical "no payment side-effect"
rationale still holds there.

## Free (fee = 0) types in payment mode

Stay Pending, exactly like approval mode. Auto-activating a $0 signup would
let a bot solve one challenge and become an Active member with portal
access; the conservative default keeps a human in the loop. If an org wants
auto-active free tiers later, that's a separate knob.
