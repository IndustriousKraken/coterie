# public-signup Specification

## MODIFIED Requirements

### Requirement: Public signup creates a Pending member

`POST /public/signup` SHALL accept new-member signup data, create a member with status `Pending`, and trigger a verification email. The endpoint SHALL be CSRF-exempt and gated by:

1. CORS allowlist (only configured origins may call it from a browser).
2. Bot challenge (Turnstile-compatible verification).

When the organization's signup mode is `approval` (the default), signup initiates no payment side-effect and is NOT covered by `money_limiter`; the bot challenge is the abuse gate. When the signup mode is `payment`, signup initiates a payment side-effect and SHALL additionally be covered by `money_limiter`, applied per the money-moving public-endpoint gate order (rate limit first).

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

### Requirement: Pending members cannot log in until verified

A member with status `Pending` SHALL NOT pass login, `require_auth_redirect`, or `require_auth`. A Pending member becomes Active via the signup mode's activation path — admin activation in `approval` mode, or a completed membership payment in `payment` mode. Email verification SHALL record `email_verified_at` and SHALL be independent of status: it does not by itself activate the member, and activation does not depend on it.

#### Scenario: Pending member is rejected at login

- **WHEN** a Pending member attempts to log in — including after completing email verification but before activation
- **THEN** login SHALL be rejected; once the member becomes Active via the mode's activation path, login SHALL succeed

#### Scenario: Unverified Pending member cannot pass auth gates

- **WHEN** a Pending member somehow obtains a session
- **THEN** `require_auth` SHALL return 403 and `require_auth_redirect` SHALL bounce to login

## ADDED Requirements

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

In payment mode, when a signup request supplies the email of an existing `Pending` member who has no completed membership payment AND the supplied password verifies against that member's password hash, the handler SHALL return a fresh checkout session for that member instead of a duplicate-email error. When the password does not verify, or the member has a completed payment or a non-Pending status, the outcome SHALL be exactly the pre-existing duplicate handling, so the retry path discloses nothing beyond what duplicate detection already does.

#### Scenario: Correct password retries the checkout

- **WHEN** a payment-mode signup repeats an email belonging to a Pending member with no completed payment and the password verifies
- **THEN** the response SHALL include a fresh checkout URL and no second member SHALL be created

#### Scenario: Wrong password gets the duplicate outcome

- **WHEN** the same repeat arrives with a password that does not verify
- **THEN** the handler SHALL respond exactly as it does for any duplicate email today, and no checkout session SHALL be created
