# Change: Migrate to rand 0.9, and specify the property rather than the call

## Why

Dependabot PR #81 bumps `rand` from 0.8 to 0.9. It cannot simply be merged, for
two reasons.

**It contradicts canon.** `email-tokens` states that plaintext tokens *"SHALL be
generated from 32 random bytes (256 bits) via the OS-RNG-backed
`rand::thread_rng()`."* In rand 0.9 `thread_rng()` no longer exists — it is
`rand::rng()`. Merging the bump would leave the code in violation of an explicit
requirement, and the requirement unsatisfiable by any version of the crate the
project could be on.

**It touches every place the application generates a secret.** `rand` is not
incidental here:

| Site | What it generates |
|---|---|
| `src/auth/tokens.rs` | email/reset token bytes |
| `src/auth/totp.rs` | TOTP shared secrets |
| `src/auth/recovery_codes.rs` | account recovery codes |
| `src/api/middleware/security_headers.rs` | CSP nonces |
| `src/service/member_service/bulk_import.rs` | generated member passwords |
| `deploy/coterie-provision/src/install/inputs.rs` | provisioning secrets |

Ten call sites across eight files, and they are the sites where a quiet mistake
is both catastrophic and invisible. Both versions' thread-local generator is a
ChaCha-based CSPRNG seeded from the OS, so the upgrade is safe in principle; what
makes it worth doing deliberately is that a mechanical rename touching this list
is exactly the kind of edit where a substitution — reaching for a seedable or
small generator because it compiles — would not show up in any test.

The deeper problem is the canon requirement itself. Naming a specific function of
a specific crate version means an upstream rename invalidates canon without
anything about the system's security actually changing. The requirement should
state the property that matters and let the implementation name the call.

## What Changes

- `rand` moves to 0.9 and the call sites move with it.
- The `email-tokens` requirement stops naming `rand::thread_rng()` and states the
  property instead: 256 bits from a cryptographically secure, OS-seeded
  generator. A future rename then changes code, not canon.
- The property is stated once and applies to every secret-generating site, not
  only to email tokens — the other five sites have the same requirement and no
  requirement recording it.
- A test asserts the property holds where it can be observed, so a substitution
  that compiles does not pass silently.

## Why this is not just a version bump

The behavior after this change is intended to be identical: same generator
family, same seeding, same output length. That is precisely why it needs stating.
A change whose success condition is "nothing observable changed" has no natural
signal for having gone wrong, and the failure mode is a system that keeps working
while producing predictable secrets.

## What this does not do

- **It does not change token lengths, formats, or lifetimes.** Only the API used
  to obtain randomness.
- **It does not switch generators.** The OS-seeded thread-local CSPRNG remains
  the source in both versions.
- **It does not touch `getrandom` directly** or introduce a second randomness
  path. One source, as now.
- **It does not resolve PR #81 as-is.** That PR is conflicted and its Cargo.toml
  change is only the first line of the work.
