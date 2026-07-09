# secure-cap-password-length

## Why

`crate::auth::validate_password` (`src/auth/mod.rs:134-148`) enforces a
**minimum** length of 10 characters and three character-class rules, but
no **maximum** length:

```rust
pub fn validate_password(password: &str) -> std::result::Result<(), &'static str> {
    if password.len() < 10 { ... }
    // ... upper / lower / digit checks ...
    Ok(())   // <-- no upper bound
}
```

It is the documented single source of truth for password rules and is
invoked on every set-password path before the result is Argon2-hashed —
including the **unauthenticated** signup (`src/api/handlers/public.rs:103`)
and password-reset (`src/web/templates/reset.rs`) flows, plus in-portal
change and first-run setup.

Argon2's input is absorbed by a Blake2b pass whose cost scales with the
password length, on top of the fixed memory-hard work. With no upper
bound, the only ceiling is axum's default request-body limit, so a caller
can submit a multi-hundred-KB "password" and force the server to hash it.
Per the public-signup spec, signup is deliberately **not** behind the
`money_limiter` (the bot challenge is its only abuse gate), so a
solved-challenge or `provider=disabled` deployment lets an attacker
amplify CPU per request — a denial-of-service vector against an
unauthenticated endpoint.

- **Trigger:** unauthenticated POST to `/public/signup` (or
  `/reset-password`) with an oversized `password` field.
- **Harm:** CPU exhaustion via Argon2 amplification on an unauthenticated
  endpoint (resource-exhaustion DoS).

This changes an observable contract — a password longer than the new
bound, previously **accepted** (`201`/redirect, member created or
password reset), is now **rejected** (`400`/inline error) at the
signup/reset/change boundary — so it lands in the spec lane as a
`MODIFIED` of the existing password-complexity requirement rather than an
issue. Capping password length (commonly 64–128 chars) is standard
hardening (OWASP) precisely to neutralize this DoS.

## What Changes

- Add a maximum-length check to `validate_password` so it rejects
  passwords longer than an upper bound (128 characters), returning a
  clear error message. Because every set-password path funnels through
  this one validator, the single guard covers signup, reset, in-portal
  change, and setup with no per-call-site edits.
- The minimum length and character-class rules are unchanged.

## Impact

- Spec: `password-management` — `MODIFIED` requirement "Password
  complexity is validated at change/reset/signup" (adds the upper-bound
  rule + a rejection scenario).
- Code: `src/auth/mod.rs` (`validate_password`).
- Tests: unit test for the new bound in `src/auth/mod.rs`.
- No database, wire-format, or API-shape changes. The 128-character cap
  is generous for passphrases; the exact number is implementation
  guidance, the binding invariant is that an upper bound exists.
