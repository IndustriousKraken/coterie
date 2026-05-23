## Why

`TotpService::confirm_enrollment` at `src/auth/totp.rs:99-131` overwrites
`totp_secret_encrypted` and stamps `totp_enabled_at` for the member as
long as the supplied `code` verifies against the supplied
`secret_base32` — it never checks whether the member already has 2FA
enabled. The web handler at
`src/web/portal/security.rs:202-275` (`enroll_confirm`) does no such
check either: `enroll_start` refuses to start a new enrollment when
`is_enabled` is true, but `enroll_confirm` is a separate endpoint that
an attacker can hit directly.

Concrete exploit, given an attacker who has a logged-in session for
member M (stolen session cookie, shared workstation, or insider with
borrowed access):

1. Attacker generates a TOTP secret in their own authenticator app and
   the current 6-digit code.
2. Attacker POSTs `/portal/profile/security/totp/enroll/confirm` with
   `secret_base32 = <their secret>`, `code = <their code>`, and the
   session's CSRF token (any rendered page on the session provides it).
3. `confirm_enrollment` verifies the code (it does match the supplied
   secret) and overwrites `members.totp_secret_encrypted` /
   `totp_enabled_at` for member M with the attacker's secret.
4. `recovery_codes::issue_for_member` (called immediately after in
   `enroll_confirm`) wipes M's old recovery codes and writes a fresh
   set, returning them in the HTML response to the attacker.
5. M's authenticator app no longer produces working codes; the
   attacker is now the only party who can complete the TOTP step at
   `/login/totp`. If the org has `auth.require_totp_for_admins` on, M
   also loses access to admin routes until they go through password
   reset and re-enrollment.

This violates the implicit invariant behind the spec at
`openspec/specs/totp-2fa/spec.md:23-40` — `confirm_enrollment` is
documented as the second leg of *enrollment*, not as a re-enrollment
or rotation primitive. The `disable` flow at
`src/web/portal/security.rs:290-330` correctly requires a current
TOTP code (or recovery code) before clearing 2FA state;
`enroll_confirm` is the back door that bypasses that requirement.

Harm: silent 2FA takeover with persistence (attacker keeps access
across session expiry / password reset until the victim notices their
codes don't work and follows the recovery path).

## What Changes

Make `TotpService::confirm_enrollment` refuse to act when the member
already has `totp_enabled_at` set, returning `Ok(false)` so the
caller's existing "code didn't match" rendering path handles the
rejection (no new error variant needed). Keep the existing
`enroll_start` guard for the UX path; the service-layer check is what
closes the direct-POST hole. Update the totp-2fa spec to make the
no-overwrite invariant explicit.

## Impact

- `src/auth/totp.rs` — `confirm_enrollment` adds an
  "already enrolled? bail" short-circuit before the UPDATE.
- `tests/totp_test.rs` — new test
  `confirm_enrollment_refuses_when_already_enabled` covering the
  invariant against a fresh in-memory pool.
- `openspec/specs/totp-2fa/spec.md` — modify the "Two-step enrollment"
  requirement to spell out that `confirm_enrollment` never overwrites
  an existing secret; rotation goes through `disable` + re-enroll.
