# rate-limiting Specification

## MODIFIED Requirements

### Requirement: Money-moving endpoints are rate-limited per IP

The system SHALL apply a per-IP rate limit (`money_limiter`) of 10 requests per 60 seconds to money-moving endpoints and to public signup. Current callers:

- `POST /public/donate` — public donation flow.
- `POST /public/signup` — both signup modes (see below).
- `POST /public/events/:id/register` — public paid-event registration (unauthenticated, initiates a Stripe Checkout session).
- `POST /portal/api/payments/checkout`, `POST /portal/api/payments/charge-saved` — portal-initiated payments.
- `POST /portal/donate` API — logged-in donations.
- `POST /portal/admin/members/:id/record-payment` — admin manual payment recording.

Adding a money-moving endpoint without wiring `money_limiter` SHALL be treated as a defect. `/public/signup` SHALL subscribe to `money_limiter` in BOTH modes, applied BEFORE the bot-challenge provider so a bursting IP cannot burn the provider's quota. In payment mode the limiter caps card-testing on the Stripe-Checkout side-effect; in approval mode it caps unauthenticated mass account creation and verification-email amplification (each signup queues a verification email), which matters because the bot challenge defaults to disabled.

`POST /public/events/:id/register` SHALL follow the same before-the-provider ordering for the same reason, and additionally caps seat-squatting: each accepted request claims a seat against the event's capacity until its payment leaves `Pending`.

#### Scenario: Donation flood is rejected

- **WHEN** an IP submits 11 donation requests within 60 seconds
- **THEN** the 11th request SHALL be rejected by the rate limiter

#### Scenario: New money endpoint must subscribe to the limiter

- **WHEN** a new endpoint that records or initiates a payment is added
- **THEN** it SHALL invoke the shared `money_limiter` and be added to the rate-limited set; reviewers SHALL block PRs that omit this

#### Scenario: Payment-mode signup shares the money limiter

- **WHEN** the org's signup mode is `payment` and an IP at the money-limiter budget submits another signup
- **THEN** the request SHALL be rejected with `429` WITHOUT consulting the bot-challenge provider

#### Scenario: Approval-mode signup is also rate-limited

- **WHEN** the org's signup mode is `approval` and an IP at the money-limiter budget submits another signup
- **THEN** the request SHALL be rejected with `429` WITHOUT consulting the bot-challenge provider (the limiter applies regardless of signup mode)

#### Scenario: Public event registration is rate-limited before the provider is consulted

- **WHEN** an IP at the money-limiter budget submits another `POST /public/events/:id/register`
- **THEN** the request SHALL be rejected with `429` WITHOUT consulting the bot-challenge provider and WITHOUT claiming a seat
