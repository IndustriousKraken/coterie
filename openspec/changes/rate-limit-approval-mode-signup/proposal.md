# rate-limit-approval-mode-signup

## Why

`POST /public/signup` is rate-limited by `money_limiter` ONLY in payment mode;
in approval mode (the default) it has no rate limiter — canon
(`rate-limiting` → "Money-moving endpoints are rate-limited per IP") explicitly
says "/public/signup in approval signup mode does NOT use this limiter." The
only remaining control is the bot challenge, which defaults to `disabled`
(a no-op verifier) and also falls back to disabled when no provider secret is
set. So an out-of-the-box deployment (approval mode + no bot-challenge provider)
has NEITHER a rate limit NOR bot protection on signup: an attacker can create
unlimited Pending members, and each signup triggers a verification email —
mass account creation plus verification-email amplification that can damage the
org's email-sender reputation.

Because canon states the approval-mode carve-out, closing it is a spec change.

## What Changes

- `POST /public/signup` SHALL be rate-limited by `money_limiter` in BOTH signup
  modes (drop the payment-mode-only guard). Approval-mode signup moves no money,
  but the per-IP cap bounds mass-signup and verification-email amplification
  even when the bot challenge is disabled. The limiter continues to run BEFORE
  the bot-challenge provider in both modes.
- No new limiter is introduced; signup joins the existing `money_limiter` set
  unconditionally.

## Impact

- Spec: `rate-limiting` — 1 MODIFIED requirement (the money-moving requirement:
  `/public/signup` becomes unconditional in the caller list and the note).
- Spec: `public-signup` — 1 MODIFIED requirement (the public-signup gate
  description: `money_limiter` now applies in BOTH modes, not payment-only;
  gate list updated to include rate limit before bot challenge).
- Code: `src/api/handlers/public.rs::signup` — remove the
  `signup_mode == SignupMode::Payment &&` guard on the `money_limiter` check.
- Tests: assert an IP over budget gets `429` on approval-mode signup too, before
  the bot-challenge verifier is consulted (extend the existing gate-order test).
