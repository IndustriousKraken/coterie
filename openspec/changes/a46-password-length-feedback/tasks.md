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

- [ ] 2.1 Add `maxlength="128"` and a visible hint to every password input on the
  four set-password surfaces: setup wizard, in-portal change
  (`templates/portal/profile*`), reset (`templates/auth/reset*`), and any signup
  form Coterie itself renders.
- [ ] 2.2 `theneontemple.com`: same on the join form's password field, which today
  carries `minlength="10"` and no upper bound. Keep the comment convention already
  there noting that the attribute mirrors Coterie's validator.
- [ ] 2.3 Note in each template that the attribute mirrors the server rule and is
  not the enforcement.

## 3. Verification

- [ ] 3.1 Manually confirm the message renders on all four paths — the incident
  that prompted this change included a report of no visible warning, and the code
  reading says one is returned on every path. Confirm empirically rather than by
  inspection.
- [ ] 3.2 Pair with `a45`: an over-length rejection should now also appear in the
  log as `auth.password_rejected`, so the next such report is answerable from the
  operator side too.
