# Tasks

The enforced ceiling does not move. Only its unit, its wording, and its
discoverability change. Do not weaken the server check to make the client hint
redundant — the hint is a convenience, the check is the control.

## 1. Validator

- [ ] 1.1 `src/auth/mod.rs`: reword the length messages to say **bytes** and to
  include the submitted size, e.g. "Password must be at most 128 bytes (yours is
  240)". The comparison itself already measures `password.len()` in bytes and is
  correct — do not change the arithmetic.
- [ ] 1.2 Apply the same wording treatment to the minimum-length message.
- [ ] 1.3 Update the existing unit tests (`accepts_128_rejects_129`, the 10_000
  case) for the new message text, and add a multi-byte case: a 60-emoji password
  is 240 bytes and must be rejected with a byte-denominated message.
- [ ] 1.4 Add a test asserting no truncation: a submission over the bound leaves
  the stored hash untouched.

## 2. Forms

- [ ] 2.1 Add a visible hint stating the 128-byte ceiling to every password input
  on the four set-password surfaces: setup wizard, in-portal change
  (`templates/portal/profile*`), reset (`templates/auth/reset*`), and any signup
  form Coterie itself renders.
- [ ] 2.2 Do NOT add `maxlength`. It silently clips pasted input on a masked
  field — a password-manager paste would be stored truncated with no visible sign,
  locking the user out of a credential they never chose — and it counts UTF-16
  code units, not bytes, so it cannot express the bound anyway.
- [ ] 2.3 Optional non-destructive client feedback: warn when the entered value
  exceeds 128 bytes, measured with `TextEncoder` so it agrees with the server.
  It must never alter or block the input.
- [ ] 2.4 Note in each template that the hint mirrors the server rule and is not
  the enforcement.

## 3. Verification

- [ ] 3.1 Add integration tests that POST an over-length password to each of the
  four set-password handlers (public signup, in-portal change, reset, setup) and
  assert the byte-denominated message appears in the response body. The incident
  that prompted this change included a report of no visible warning while the code
  reading says one is returned on every path — so the point is empirical
  confirmation on all four, not inspection.
- [ ] 3.2 Pair with `a45`: an over-length rejection should now also appear in the
  log as `auth.password_rejected`, so the next such report is answerable from the
  operator side too.

## 4. Companion (marketing repo, not this spec)

- [ ] 4.1 `theneontemple.com` join form
  (`themes/terminal/layouts/_default/join.html`): add a visible hint stating the
  128-byte ceiling next to the password input, and extend the adjacent comment to
  say the hint mirrors Coterie's `validate_password` bounds (10 and 128 bytes) and
  is a convenience rather than the enforcement. Do NOT add `maxlength` — it clips
  pasted input silently on a masked field and counts UTF-16 code units rather than
  bytes. The existing `minlength="10"` is unaffected; it does not truncate.
