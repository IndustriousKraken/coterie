# Tasks

The enforced ceiling does not move. Only its unit, its wording, and its
discoverability change. Do not weaken the server check to make the client hint
redundant — the hint is a convenience, the check is the control.

## 1. Validator

- [x] 1.1 `src/auth/mod.rs`: reword the length messages to say **bytes** and to
  include the submitted size, e.g. "Password must be at most 128 bytes (yours is
  240)". The comparison itself already measures `password.len()` in bytes and is
  correct — do not change the arithmetic.
- [x] 1.2 Apply the same wording treatment to the minimum-length message.
- [x] 1.3 Update the existing unit tests (`accepts_128_rejects_129`, the 10_000
  case) for the new message text, and add a multi-byte case: a 60-emoji password
  is 240 bytes and must be rejected with a byte-denominated message.
- [x] 1.4 Add a test asserting no truncation: a submission over the bound leaves
  the stored hash untouched.
  (Needs a DB, so it lives in `tests/password_length_feedback_test.rs`:
  `in_portal_change_reports_the_byte_bound_and_leaves_the_hash_untouched`
  compares `members.password_hash` before and after; the reset case asserts the
  same and that the token survives.)

## 2. Forms

- [x] 2.1 Add a visible hint stating the 128-byte ceiling to every password input
  on the four set-password surfaces: setup wizard, in-portal change
  (`templates/portal/profile*`), reset (`templates/auth/reset*`), and any signup
  form Coterie itself renders.
  (Coterie renders no signup form of its own — `/public/signup` is JSON — so the
  signup surface here is the three `examples/*/join.html` reference forms, whose
  requirement lists now state the 10-128 byte bound. Their `minlength` was 8,
  disagreeing with the server's 10; corrected. `templates/admin/member_new.html`
  is deliberately untouched — see the follow-up note below.)
- [x] 2.2 Do NOT add `maxlength`. It silently clips pasted input on a masked
  field — a password-manager paste would be stored truncated with no visible sign,
  locking the user out of a credential they never chose — and it counts UTF-16
  code units, not bytes, so it cannot express the bound anyway.
- [x] 2.3 Optional non-destructive client feedback: warn when the entered value
  exceeds 128 bytes, measured with `TextEncoder` so it agrees with the server.
  It must never alter or block the input.
  (One nonce'd listener in `templates/layouts/base.html`, opt-in per field via
  `data-max-bytes`; it appends a warning paragraph and never touches the value.)
- [x] 2.4 Note in each template that the hint mirrors the server rule and is not
  the enforcement.

## 3. Verification

- [x] 3.1 Add integration tests that POST an over-length password to each of the
  four set-password handlers (public signup, in-portal change, reset, setup) and
  assert the byte-denominated message appears in the response body. The incident
  that prompted this change included a report of no visible warning while the code
  reading says one is returned on every path — so the point is empirical
  confirmation on all four, not inspection.
  (`tests/password_length_feedback_test.rs`, four tests. The probe password is
  "Aa1" + 60 emoji: 63 characters, 243 bytes, so a character-denominated message
  would be visibly wrong rather than merely imprecise.)
- [x] 3.2 Pair with `a45`: an over-length rejection should now also appear in the
  log as `auth.password_rejected`, so the next such report is answerable from the
  operator side too.
  (`tests/auth_logging_test.rs`:
  `an_over_length_password_is_logged_as_too_long_with_its_byte_count` — asserts
  `reason=too_long` and `length=243`, the same byte count the user is shown.)
