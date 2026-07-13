# Signup-retry password check is a timing oracle for Pending-unpaid emails

## Summary

The payment-mode signup retry path (`retry_pending_checkout`,
`src/api/handlers/public.rs:402`) is designed so that a duplicate signup
discloses nothing beyond what plain duplicate detection already reveals: every
non-resuming case returns the identical `409 Conflict` with the identical
generic body. Status and body are indeed constant. **Timing is not.** The
Argon2 password verification runs only on ONE branch — an existing member who
is `Pending` with no completed payment — and the other early-return branches
(email not found / username-only collision, non-`Pending` status) return before
any hashing. Argon2 is deliberately expensive (tens of ms), so a request whose
email belongs to a Pending-unpaid signup takes measurably longer than one whose
email is Active/Expired/Suspended or is a username collision. An attacker can
therefore distinguish "this email is a Pending-unpaid signup" from other
registered emails by response latency alone, among otherwise-identical `409`s.

## Source location

`src/api/handlers/public.rs:413-428` (branch ordering in
`retry_pending_checkout`):

```rust
    let Some(member) = member_repo.find_by_email(email).await? else {
        return Ok(None);                 // (a) no email match — FAST, no hash
    };
    if member.status != MemberStatus::Pending {
        return Ok(None);                 // (b) wrong status — FAST, no hash
    }
    let Some(hash) = crate::auth::get_password_hash(db_pool, email).await? else {
        return Ok(None);
    };
    if !AuthService::verify_password(password, &hash).await.unwrap_or(false) {
        return Ok(None);                 // (c) wrong password — SLOW, argon2 ran
    }
```

Only path (c) pays the Argon2 cost; (a) and (b) return early.

## Why this is harmful

Gated behind `money_limiter` (10/min per IP), so exploitation is slow, and the
disclosed fact (an email is a Pending-unpaid signup) is low-sensitivity — but it
is a genuine violation of the endpoint's stated anti-disclosure invariant and a
member-enumeration side-channel. (Note: this issue does not depend on, and is
compounded by, the separate X-Forwarded-For rate-limit-bypass issue.)

- **Trigger:** repeated payment-mode `/public/signup` submissions with a target
  email and a wrong password, comparing response latency.
- **Harm:** timing side-channel enumerating Pending-unpaid member emails.

## Acceptance criteria (against existing specification)

This corrects code to meet an invariant already stated in canon; no contract
changes.

- **`public-signup` → Requirement "Abandoned signup checkout is retryable":**
  "...the retry path discloses nothing beyond what duplicate detection already
  does." Timing that distinguishes a Pending-unpaid email from other registered
  emails discloses more than duplicate detection (a boolean email-exists signal)
  does. After the fix, the retry path's timing MUST NOT distinguish the
  password-verified branch from the early-return branches.

Concretely:

1. `retry_pending_checkout` MUST perform an Argon2 verification of equivalent
   cost on the early-return branches (email-not-found, wrong-status) — e.g. call
   the existing `AuthService::verify_dummy(password)` (a full hash against a
   fixed dummy hash, already used by the login handler for exactly this
   anti-enumeration purpose) before returning `Ok(None)` — so all `409`-yielding
   paths incur the same hashing cost.
2. The observable response (status `409`, generic body) is unchanged; only the
   timing is equalized. The resuming path (correct password → checkout URL) is
   unchanged.

## Tasks

- [ ] 1.1 In `src/api/handlers/public.rs::retry_pending_checkout`, on the
  email-not-found and non-`Pending`-status early returns, run
  `AuthService::verify_dummy(password).await` before `return Ok(None)` so those
  branches incur the same Argon2 cost as the wrong-password branch. Confirm
  `verify_dummy` exists (`src/auth/mod.rs:57`) and is the same primitive the
  login handler uses for enumeration-resistance.
- [ ] 2.1 Add a test asserting the response for a duplicate signup with a wrong
  password, a non-existent email, and a non-Pending member is byte-identical
  (`409`, same body) — the existing behavioral guarantee — and note in a comment
  that the dummy-verify equalizes timing (timing itself is not asserted, but the
  dummy-verify call site is covered).
- [ ] 3.1 Run `cargo test --features test-utils --test pay_at_signup_test` and
  confirm the retry-rule tests still pass.
