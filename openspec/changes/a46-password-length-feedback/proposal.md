# a46-password-length-feedback

## Why

A security tester tried to set a 200-character password and came away unable to
tell what had happened. Investigating that report turned up one reassurance and
one real defect.

**The reassurance, stated plainly because the opposite was suspected:** the
password is **not** silently truncated. Coterie hashes with Argon2, not bcrypt, so
there is no 72-byte cliff, and `validate_password` rejects an over-long password
outright *before* hashing. All four set-password paths (public signup, in-portal
change, reset, setup) call the validator, every one returns the rejection message,
and both the marketing form and the portal render it. A password set at 200
characters is refused, never quietly shortened, so nobody ends up with an account
whose real password is a prefix of what they typed.

**The real defect:** the bound is measured in **bytes** and described in
**characters**. `validate_password` tests `password.len()`, which in Rust is the
UTF-8 byte length, against a message that says "at most 128 characters". For an
ASCII password those agree. For anything else they do not — and "anything else" is
exactly what a security tester types. A 60-character password of emoji is 240
bytes and is refused as being over "128 characters", which is not true and reads
as the system malfunctioning. That mismatch is a plausible explanation for a
report of "it didn't warn me properly".

**The secondary gap:** no password field carries a `maxlength`, so the browser
offers no feedback at all. The user types 200 characters, submits, waits, and only
then learns there was a ceiling. The ceiling is a legitimate anti-DoS control —
Argon2's pre-hash cost scales with input length — but a control the user only
discovers by tripping it is a bad control.

Logging of these rejections is deliberately **not** in this change; it belongs
with the rest of the auth logging in `a45-auth-security-logging`.

## What Changes

- **The bound becomes byte-denominated in both the check and the message.** The
  limit stays 128 bytes (the DoS argument is about bytes fed to Argon2, so bytes
  are the correct unit), and the message stops claiming characters. Non-ASCII
  users get a message that matches what was actually measured.
- **The message states the ceiling and what was submitted**, so a user who trips
  it can act on it instead of guessing which of their 200 characters were too
  many.
- **Password inputs gain a visible hint** stating the ceiling, so it is
  discoverable before submission rather than after. The hint is a convenience,
  never the enforcement — the server check remains authoritative and unchanged in
  strength.
- **Deliberately no `maxlength`.** It was in an earlier draft and is wrong twice
  over: it silently clips pasted input on a masked field, which would store a
  truncated password-manager paste with no visible sign and lock the user out of a
  credential they never chose — reintroducing at the client precisely the silent
  truncation this change rules out at the server. It also counts UTF-16 code
  units rather than bytes, so it cannot express the bound for non-ASCII input
  anyway. Any client-side length feedback must warn without altering the value.
- **The minimum-length message gets the same treatment** for consistency, since it
  has the same byte-vs-character ambiguity.

## Impact

- **Spec:** `password-management` — 1 MODIFIED requirement (the complexity
  requirement, which currently says "128 characters" and now must say bytes and
  require actionable feedback).
- **Code:** `src/auth/mod.rs` (`validate_password` message wording; the comparison
  itself already measures bytes and is correct as-is), the four set-password form
  templates, and `theneontemple.com`'s join form for the `maxlength` attribute.
- **Not a security regression:** the enforced ceiling does not move. Only its
  description and its discoverability change.
- **Deferred:** a password-strength meter, breached-password checking against a
  corpus, and raising the ceiling. Raising it would need a fresh look at the
  Argon2 amplification budget, which this change does not attempt.
