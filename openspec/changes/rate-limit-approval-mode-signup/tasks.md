# Tasks

## 1. Apply the limiter unconditionally

- [ ] 1.1 In `src/api/handlers/public.rs::signup`, remove the
  `signup_mode == SignupMode::Payment &&` condition so
  `money_limiter.0.check_and_record(ip)` runs for every signup; keep it BEFORE
  the bot-challenge verification in both modes (the money-limiter-first
  ordering is unchanged).

## 2. Tests

- [ ] 2.1 Add/extend a test: in APPROVAL mode, an IP at the money-limiter
  budget gets `429` on the next signup WITHOUT the bot-challenge verifier being
  consulted (mirror the existing payment-mode
  `payment_mode_rate_limit_precedes_bot_challenge` test with a counting deny
  verifier).
- [ ] 2.2 Confirm the payment-mode gate-order test still passes unchanged.
