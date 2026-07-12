# rate-limiting Specification

## MODIFIED Requirements

### Requirement: Money-moving endpoints are rate-limited per IP

The system SHALL apply a per-IP rate limit (`money_limiter`) of 10 requests per 60 seconds to money-moving endpoints. Current callers:

- `POST /public/donate` — public donation flow.
- `POST /public/signup` — in payment signup mode only (see below).
- `POST /portal/api/payments/checkout`, `POST /portal/api/payments/charge-saved` — portal-initiated payments.
- `POST /portal/donate` API — logged-in donations.
- `POST /portal/admin/members/:id/record-payment` — admin manual payment recording.

Adding a money-moving endpoint without wiring `money_limiter` SHALL be treated as a defect. Note: `/public/signup` in approval signup mode does NOT use this limiter (signup then creates a Pending member with no payment side-effect; its abuse-control is bot challenge + CORS only). In payment signup mode `/public/signup` initiates a Stripe Checkout session and SHALL subscribe to the shared `money_limiter`, applied BEFORE the bot-challenge provider so a bursting IP cannot burn the provider's quota.

#### Scenario: Donation flood is rejected

- **WHEN** an IP submits 11 donation requests within 60 seconds
- **THEN** the 11th request SHALL be rejected by the rate limiter

#### Scenario: New money endpoint must subscribe to the limiter

- **WHEN** a new endpoint that records or initiates a payment is added
- **THEN** it SHALL invoke the shared `money_limiter` and be added to the rate-limited set; reviewers SHALL block PRs that omit this

#### Scenario: Payment-mode signup shares the money limiter

- **WHEN** the org's signup mode is `payment` and an IP at the money-limiter budget submits another signup
- **THEN** the request SHALL be rejected with `429` WITHOUT consulting the bot-challenge provider; in approval mode the same request is not money-limited
