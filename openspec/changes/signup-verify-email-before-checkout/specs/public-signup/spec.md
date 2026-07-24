# public-signup Specification

## MODIFIED Requirements

### Requirement: Payment-mode signup initiates Stripe checkout

When the signup mode is `payment` and the resolved membership type has a fee greater than zero, `POST /public/signup` SHALL create the Pending member and queue the verification email, but SHALL NOT create a Stripe customer or Checkout session at signup time and SHALL NOT return a checkout URL. The success response SHALL instruct the caller to verify their email to continue to payment. The Stripe Checkout session is initiated only AFTER the member verifies their email (see "Email verification initiates payment-mode checkout"), using the same session contract as portal dues checkout (metadata: `member_id`, `payment_type=membership`, `membership_type_slug`). A membership type with a fee of zero SHALL behave as in approval mode: the member stays Pending and no checkout session is created. Deferring the Stripe surface behind a verifiable inbox prevents automated card-testing signups (fake/unmonitored addresses) from ever reaching checkout.

#### Scenario: Paid type queues verification instead of checkout

- **WHEN** a payment-mode signup resolves a membership type with `fee_cents > 0`
- **THEN** the response SHALL instruct the caller to verify their email and SHALL NOT include a checkout URL, and NO Stripe customer or Checkout session SHALL be created at signup time

#### Scenario: Free type stays in the approval funnel

- **WHEN** a payment-mode signup resolves a membership type with `fee_cents == 0`
- **THEN** no checkout session SHALL be created and the member SHALL remain Pending

#### Scenario: Rate limit precedes bot challenge in payment mode

- **WHEN** an IP at the money-limiter budget submits another payment-mode signup
- **THEN** the handler SHALL return 429 WITHOUT calling the bot-challenge provider

### Requirement: Abandoned signup checkout is retryable

In payment mode, when a signup request supplies the email of an existing `Pending` member who has no completed membership payment AND the supplied password verifies against that member's password hash AND that member's email is verified, the handler SHALL return a checkout session for that member instead of a duplicate-email error. When the member's most recent pending checkout session is still open on Stripe, the handler SHALL return THAT session's URL rather than creating a new one — a retry does not accumulate duplicate pending payment rows or leave multiple payable sessions open. When the previous session is no longer open (expired or unretrievable), its Pending payment row SHALL be marked Failed before a fresh session is created. When the member's email is NOT yet verified, the handler SHALL re-queue the verification email and SHALL NOT create a checkout session, so an unverified retry cannot reach Stripe. When the password does not verify, or the member has a completed payment or a non-Pending status, the outcome SHALL be exactly the pre-existing duplicate handling, so the retry path discloses nothing beyond what duplicate detection already does.

#### Scenario: Correct password resumes the open checkout

- **WHEN** a payment-mode signup repeats an email belonging to a Pending member with no completed payment and a verified email, the password verifies, and the member's previous checkout session is still open
- **THEN** the response SHALL carry the existing session's URL, no second member SHALL be created, and no additional pending payment row SHALL be written

#### Scenario: Expired previous session is superseded, not orphaned

- **WHEN** the same retry arrives (verified email, correct password) but the previous checkout session is no longer open
- **THEN** the previous session's Pending payment row SHALL be marked Failed and the response SHALL carry a fresh checkout session's URL

#### Scenario: Unverified retry re-queues verification, no checkout

- **WHEN** a retry arrives for a Pending member whose email is NOT yet verified (correct password)
- **THEN** the handler SHALL re-queue the verification email and SHALL NOT create a checkout session

#### Scenario: Wrong password gets the duplicate outcome

- **WHEN** the same repeat arrives with a password that does not verify
- **THEN** the handler SHALL respond exactly as it does for any duplicate email today, and no checkout session SHALL be created

## ADDED Requirements

### Requirement: Email verification initiates payment-mode checkout

Email verification SHALL be the point at which a payment-mode signup reaches Stripe. When a member with status `Pending` who has no completed membership payment verifies their email, AND the signup mode is `payment` AND their membership type fee is greater than zero, the verification handler SHALL mark the email verified and THEN initiate a Stripe Checkout session for that member — using the same session contract and auto-renew handling as any signup checkout — and direct the member to it. For approval mode, a fee-of-zero type, or an already-verified member, verification SHALL retain its prior behavior (mark verified; the member awaits admin review) and SHALL NOT create a checkout session. A Stripe Checkout session for a signup SHALL only ever be created for a member whose email is verified.

#### Scenario: Verifying a paid signup opens checkout

- **WHEN** a Pending, unpaid, payment-mode member whose membership type fee is greater than zero clicks their verification link
- **THEN** the member's email SHALL be marked verified AND a Stripe Checkout session SHALL be initiated for them and presented for payment

#### Scenario: Approval-mode verification still awaits review

- **WHEN** a Pending member in approval mode (or with a fee-of-zero type) verifies their email
- **THEN** the member SHALL be marked verified and shown the awaiting-review result, and NO checkout session SHALL be created

#### Scenario: A checkout is never created for an unverified member

- **WHEN** any signup or retry path would create a Stripe Checkout session for a member whose email is not verified
- **THEN** no session SHALL be created; the verification email is the required precondition
