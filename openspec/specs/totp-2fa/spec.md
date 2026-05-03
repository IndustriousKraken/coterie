# totp-2fa Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: TOTP uses RFC 6238 standard parameters

The system SHALL implement TOTP per RFC 6238 with the parameters every popular authenticator app expects:

- Algorithm: SHA-1
- Digits: 6
- Step: 30 seconds
- Skew: ±1 step (~30s clock-skew tolerance on each side)
- Secret length: 20 bytes (160 bits)

Deviating from these parameters SHALL be forbidden because it silently breaks compatibility with Google Authenticator, Authy, 1Password, and similar apps.

#### Scenario: Code from authenticator app validates within ±30s skew

- **WHEN** a code generated for time `T` is submitted at time `T - 30s` to `T + 30s`
- **THEN** the verification SHALL succeed; outside that window SHALL fail

### Requirement: Two-step enrollment with no DB write before confirmation

Enrollment SHALL be:

1. `begin_enrollment(account_name)` — generates a fresh 20-byte secret in memory and returns the otpauth:// URL, base32 secret, and an SVG QR code. **No database write.**
2. `confirm_enrollment(member_id, secret_base32, code, account_name)` — verifies the supplied code matches the secret, then writes encrypted secret + `totp_enabled_at` to the `members` row.

The plaintext secret SHALL round-trip through the enrollment page in a hidden field; this is the only path it takes between begin and confirm.

#### Scenario: Abandoned enrollment leaves no DB trace

- **WHEN** a member starts enrollment but never submits the confirmation code
- **THEN** the database SHALL contain no record of the started enrollment

#### Scenario: Confirmation with valid code persists encrypted secret

- **WHEN** `confirm_enrollment` receives a code that verifies against the supplied secret
- **THEN** the row's `totp_secret_encrypted` and `totp_enabled_at` columns SHALL be set in a single UPDATE; recovery-code generation SHALL be a separate caller-driven step

### Requirement: Secrets are encrypted at rest with ChaCha20-Poly1305

The TOTP secret SHALL be encrypted using `SecretCrypto`, which uses ChaCha20-Poly1305 AEAD with a key derived from the application's `session_secret`. The same encryption key SHALL be used for SMTP passwords and Discord secrets.

#### Scenario: Database row stores ciphertext only

- **WHEN** an enrolled member's `members` row is read directly
- **THEN** the `totp_secret_encrypted` column SHALL be ciphertext; the base32 plaintext SHALL never appear

#### Scenario: Decryption failure (after secret rotation) is treated as "not enrolled"

- **WHEN** the application's `session_secret` is rotated and a previously-encrypted secret cannot decrypt
- **THEN** `verify_for_member` SHALL return `Ok(false)` rather than propagating the decryption error; the member effectively becomes "not enrolled" for login purposes

### Requirement: `is_enabled` reads only `totp_enabled_at`

The "is 2FA on for this member?" check SHALL be a query against `totp_enabled_at` on the `members` row. The login flow SHALL NOT need to decrypt the secret to make this decision.

#### Scenario: Login can route to /login/totp without decrypting

- **WHEN** the login handler decides whether to require a second factor
- **THEN** it SHALL only call `is_enabled(member_id)`, which SHALL read `totp_enabled_at` and return whether it is non-NULL

### Requirement: Disable wipes secret, enrolled-at, recovery codes, and pending logins

`disable(member_id)` SHALL clear `totp_secret_encrypted`, `totp_enabled_at`, AND `totp_recovery_codes` on the `members` row, AND delete all rows in `pending_logins` for that member, all in a single transaction.

#### Scenario: Disable is atomic across columns and pending_logins

- **WHEN** an authenticated member disables TOTP
- **THEN** the `members` row's three TOTP columns AND any `pending_logins` rows SHALL be cleared in one transaction; a partial disable is forbidden

#### Scenario: Disable is an authenticated action

- **WHEN** the disable handler is reached
- **THEN** the caller MUST have already verified a current TOTP code or recovery code; the underlying `disable` method does NOT itself check authentication

### Requirement: Admin TOTP enforcement is opt-in via setting

When the `auth.require_totp_for_admins` setting is `true`, admin-only routes (gated by `require_admin_redirect`) SHALL require the requesting admin to have `totp_enabled_at` set. Without TOTP, the admin SHALL be redirected to `/portal/profile/security?reason=admin_totp_required` and SHALL retain member-side access.

#### Scenario: Setting toggle takes effect without restart

- **WHEN** an operator flips `auth.require_totp_for_admins` from `false` to `true`
- **THEN** subsequent admin-route requests SHALL evaluate the new setting on each hit

#### Scenario: Admin without TOTP keeps member access

- **WHEN** the toggle is on and an admin without TOTP visits `/portal/dashboard`
- **THEN** the request SHALL succeed because member routes do not enforce admin TOTP

