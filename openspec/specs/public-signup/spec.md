# public-signup Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Public signup creates a Pending member

`POST /public/signup` SHALL accept new-member signup data, create a member with status `Pending`, and trigger a verification email. The endpoint SHALL be CSRF-exempt and gated by:

1. CORS allowlist (only configured origins may call it from a browser).
2. `money_limiter` (per-IP rate limit, applied in BOTH signup modes).
3. Bot challenge (Turnstile-compatible verification).

`money_limiter` SHALL run BEFORE the bot-challenge provider in both modes so a bursting IP cannot burn the provider's quota. When the organization's signup mode is `approval` (the default), signup initiates no payment side-effect but SHALL still be covered by `money_limiter` to cap mass account creation and verification-email amplification. When the signup mode is `payment`, signup initiates a payment side-effect and the same `money_limiter` applies per the money-moving public-endpoint gate order.

The endpoint SHALL be documented in `src/api/docs.rs` so the OpenAPI spec stays accurate.

#### Scenario: Successful signup returns 200 and emails verification

- **WHEN** valid signup data with a verified bot-challenge token reaches `/public/signup`
- **THEN** a new `Pending` member SHALL be persisted and a verification email SHALL be queued

#### Scenario: Missing bot token fails closed

- **WHEN** the bot-challenge provider is configured and the request omits the token
- **THEN** the request SHALL be rejected with 403 before signup logic runs

#### Scenario: Cross-origin from non-allowlisted origin is blocked

- **WHEN** a browser at a non-allowlisted origin attempts a cross-origin POST
- **THEN** the browser SHALL block it via the CORS policy

#### Scenario: Bot challenge runs before any database work

- **WHEN** a signup request reaches the handler with a missing or invalid token
- **THEN** the handler SHALL return 403 BEFORE any membership-type lookup or member creation, so an attacker cannot use signup to probe internal state

#### Scenario: Approval-mode signup is rate-limited

- **WHEN** the org's signup mode is `approval` and an IP at the money-limiter budget submits another signup
- **THEN** the request SHALL be rejected with `429` WITHOUT consulting the bot-challenge provider

### Requirement: Pending members cannot log in until verified

A member with status `Pending` SHALL NOT pass login, `require_auth_redirect`, or `require_auth`. A Pending member becomes Active via the signup mode's activation path — admin activation in `approval` mode, or a completed membership payment in `payment` mode. Email verification SHALL record `email_verified_at` and SHALL be independent of status: it does not by itself activate the member, and activation does not depend on it.

#### Scenario: Pending member is rejected at login

- **WHEN** a Pending member attempts to log in — including after completing email verification but before activation
- **THEN** login SHALL be rejected; once the member becomes Active via the mode's activation path, login SHALL succeed

#### Scenario: Unverified Pending member cannot pass auth gates

- **WHEN** a Pending member somehow obtains a session
- **THEN** `require_auth` SHALL return 403 and `require_auth_redirect` SHALL bounce to login

### Requirement: Signup bounds and validates its input fields

`POST /public/signup` SHALL validate and length-bound its free-text input fields before creating a member, so an unauthenticated caller cannot persist unbounded data. The handler SHALL reject the request with `400` (`AppError::BadRequest`) when any of the following fails (checked against the trimmed value):

- `email`: non-empty, contains `@`, and at most 254 characters.
- `full_name`: non-empty and at most 200 characters.
- `username`: non-empty and at most 100 characters.

These bounds match the existing public-donate handler so the two unauthenticated entry points are consistent. Validation SHALL run after the bot-challenge gate and before any member is persisted.

#### Scenario: Over-long field is rejected

- **WHEN** a signup request supplies an `email` longer than 254 characters, a `full_name` longer than 200 characters, or a `username` longer than 100 characters
- **THEN** the handler SHALL return `400` and SHALL NOT create a member

#### Scenario: Empty required field is rejected

- **WHEN** a signup request supplies an empty or whitespace-only `username` or `full_name`
- **THEN** the handler SHALL return `400` and SHALL NOT create a member

#### Scenario: Valid bounded signup succeeds

- **WHEN** a signup request supplies a valid `@`-bearing email within 254 characters and non-empty `username`/`full_name` within their bounds, with a verified bot-challenge token
- **THEN** a `Pending` member SHALL be created (the bounds do not reject normal-length input)

### Requirement: Signup mode is an organization setting

The organization SHALL have a `membership.signup_mode` setting with values `approval` (default) and `payment`, stored in `app_settings` under the `membership` category so it is editable on the admin settings page. A missing or unrecognized value SHALL behave as `approval`.

#### Scenario: Default is approval mode

- **WHEN** a deployment has no explicit `membership.signup_mode` value
- **THEN** signup SHALL behave exactly as the pre-existing approval funnel (Pending member, verification email, no payment side-effect, admin activation)

#### Scenario: Admin can switch modes at runtime

- **WHEN** an admin sets `membership.signup_mode` to `payment` via the settings page
- **THEN** subsequent signups SHALL follow the payment-mode funnel without a restart

### Requirement: Payment-mode signup initiates Stripe checkout

When the signup mode is `payment` and the resolved membership type has a fee greater than zero, `POST /public/signup` SHALL — after creating the Pending member and queueing the verification email — create a Stripe Checkout session for that member and membership type using the same session contract as portal dues checkout (metadata: `member_id`, `payment_type=membership`, `membership_type_slug`), and SHALL return the checkout URL in the success response for the caller to redirect to. A membership type with a fee of zero SHALL behave as in approval mode: the member stays Pending and no checkout session is created.

#### Scenario: Paid type returns a checkout URL

- **WHEN** a payment-mode signup resolves a membership type with `fee_cents > 0`
- **THEN** the response SHALL include a Stripe Checkout URL whose session metadata carries `member_id`, `payment_type=membership`, and the type's `membership_type_slug`

#### Scenario: Free type stays in the approval funnel

- **WHEN** a payment-mode signup resolves a membership type with `fee_cents == 0`
- **THEN** no checkout session SHALL be created and the member SHALL remain Pending

#### Scenario: Rate limit precedes bot challenge in payment mode

- **WHEN** an IP at the money-limiter budget submits another payment-mode signup
- **THEN** the handler SHALL return 429 WITHOUT calling the bot-challenge provider

### Requirement: Completed membership payment activates a Pending member

A completed membership payment for a member with status `Pending` SHALL transition the member to `Active` as part of the same atomic dues-extension claim that already revives `Expired` members, regardless of signup mode. The transition SHALL dispatch the same member-activated integration event as admin activation, so integrations observe payment-activated and admin-activated members identically.

#### Scenario: Checkout completion activates the signup

- **WHEN** the `checkout.session.completed` webhook records a completed membership payment for a Pending member
- **THEN** the member SHALL become Active with `dues_paid_until` extended, and the member-activated integration event SHALL be dispatched

#### Scenario: Admin-recorded dues activate a Pending member

- **WHEN** an admin records a completed manual membership payment for a Pending member
- **THEN** the member SHALL become Active, identically to the webhook path

### Requirement: Abandoned signup checkout is retryable

In payment mode, when a signup request supplies the email of an existing `Pending` member who has no completed membership payment AND the supplied password verifies against that member's password hash, the handler SHALL return a checkout session for that member instead of a duplicate-email error. When the member's most recent pending checkout session is still open on Stripe, the handler SHALL return THAT session's URL rather than creating a new one — a retry does not accumulate duplicate pending payment rows or leave multiple payable sessions open. When the previous session is no longer open (expired or unretrievable), its Pending payment row SHALL be marked Failed before a fresh session is created. When the password does not verify, or the member has a completed payment or a non-Pending status, the outcome SHALL be exactly the pre-existing duplicate handling, so the retry path discloses nothing beyond what duplicate detection already does.

#### Scenario: Correct password resumes the open checkout

- **WHEN** a payment-mode signup repeats an email belonging to a Pending member with no completed payment, the password verifies, and the member's previous checkout session is still open
- **THEN** the response SHALL carry the existing session's URL, no second member SHALL be created, and no additional pending payment row SHALL be written

#### Scenario: Expired previous session is superseded, not orphaned

- **WHEN** the same retry arrives but the previous checkout session is no longer open
- **THEN** the previous session's Pending payment row SHALL be marked Failed and the response SHALL carry a fresh checkout session's URL

#### Scenario: Wrong password gets the duplicate outcome

- **WHEN** the same repeat arrives with a password that does not verify
- **THEN** the handler SHALL respond exactly as it does for any duplicate email today, and no checkout session SHALL be created

### Requirement: Paid signups enroll in auto-renew by default

The organization SHALL have a `membership.signup_auto_renew` boolean setting (default `true`), consulted at signup-checkout creation in payment mode. When enabled, the signup checkout session SHALL be created against a Stripe customer for the member with the card saved for off-session use, and — upon the completed payment — the member SHALL be enrolled in auto-renew: the paying card stored as a saved card (de-duplicated by card fingerprint, becoming the default when the member has none), the Stripe customer recorded on the member, billing mode set to Coterie-managed, and the next renewal scheduled from the newly extended dues date. Enrollment failures SHALL NOT fail the payment or the webhook — the member stays Active with dues extended, and the failure is logged. When the setting is disabled, signup payment behaves as a one-off charge: no customer requirement, no card saved, billing mode untouched.

#### Scenario: Signup payment enrolls the member in auto-renew

- **WHEN** `membership.signup_auto_renew` is enabled and a payment-mode signup's checkout completes
- **THEN** the member SHALL have the paying card saved (default if they had none), billing mode `coterie_managed`, and a pending scheduled payment due at their new `dues_paid_until`

#### Scenario: Setting disabled keeps one-off semantics

- **WHEN** `membership.signup_auto_renew` is disabled and a payment-mode signup's checkout completes
- **THEN** no card SHALL be saved and the member's billing mode SHALL remain `manual`

#### Scenario: Enrollment failure does not fail the payment

- **WHEN** the post-payment enrollment step errors (e.g. the card listing fails)
- **THEN** the member SHALL still be Active with dues extended, the webhook SHALL succeed, and the failure SHALL be logged

### Requirement: Signup rejects unknown or inactive membership types

`POST /public/signup` SHALL reject a supplied `membership_type_slug` that does not resolve to an ACTIVE membership type. An unknown slug SHALL be rejected with `400`; a slug that resolves to a membership type whose `is_active` flag is false SHALL ALSO be rejected with `400`, before any member is created — a deactivated type is not signup-able even though it still exists in the database. An omitted slug SHALL take the organization's default (the first active membership type by sort order), and a known, active slug SHALL be accepted unchanged.

#### Scenario: Inactive membership-type slug is rejected

- **WHEN** a signup supplies a `membership_type_slug` that exists but whose type is inactive
- **THEN** the handler SHALL return `400` and SHALL NOT create a member

#### Scenario: Unknown membership-type slug is rejected

- **WHEN** a signup supplies a `membership_type_slug` that matches no membership type
- **THEN** the handler SHALL return `400` and SHALL NOT create a member

#### Scenario: Omitted slug takes the org default

- **WHEN** a signup omits `membership_type_slug`
- **THEN** the member SHALL be created on the organization's default (first active) membership type

