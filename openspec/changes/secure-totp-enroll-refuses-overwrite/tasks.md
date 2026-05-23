## 1. Service-layer guard against overwriting existing TOTP secret

- [ ] 1.1 In `src/auth/totp.rs::TotpService::confirm_enrollment`,
  before calling `decode_base32` / `build_totp` / `check_code`, query
  the member's current `totp_enabled_at` (a `SELECT totp_enabled_at
  FROM members WHERE id = ?` round-trip, or reuse `self.is_enabled`).
  If it is non-null, return `Ok(false)` immediately — do NOT decode
  the secret, do NOT verify the code, do NOT touch the row.
- [ ] 1.2 Add a doc comment to `confirm_enrollment` that explains
  the invariant: this is the second leg of *initial* enrollment;
  rotation goes through `disable` first. Reference the spec.

## 2. Test coverage for the no-overwrite invariant

- [ ] 2.1 In `tests/totp_test.rs`, add a test
  `confirm_enrollment_refuses_when_already_enabled` that:
  - sets up an in-memory pool via the existing helpers,
  - calls `confirm_enrollment` once to enroll the member with secret A,
  - calls `confirm_enrollment` again with a different secret B and a
    code valid against B,
  - asserts the second call returns `Ok(false)` and that
    `verify_for_member` still succeeds for a code generated from
    secret A (i.e., secret A is unchanged in the row).
- [ ] 2.2 Re-run the existing TOTP tests to confirm the "fresh
  enrollment" happy path still passes (the guard only fires when
  `totp_enabled_at` is non-null).

## 3. Spec update locking in the invariant

- [ ] 3.1 In `openspec/specs/totp-2fa/spec.md`, under the "Two-step
  enrollment with no DB write before confirmation" requirement, add
  a `#### Scenario: confirm_enrollment refuses to overwrite an
  existing secret` describing the rejection: an already-enrolled
  member who submits `confirm_enrollment` SHALL NOT have their
  `totp_secret_encrypted` or `totp_enabled_at` columns mutated.
- [ ] 3.2 Add a one-sentence cross-reference to the "Disable wipes
  …" requirement so a reader knows the rotation path: `disable` then
  re-enroll.
