## ADDED Requirements

### Requirement: Public donations route through Stripe

`POST /public/donate` SHALL initiate a donation flow via Stripe (e.g., Checkout Session creation) and persist the resulting donation intent. The endpoint SHALL be CSRF-exempt and gated by:

1. CORS allowlist.
2. Per-IP rate limit (`money_limiter`).
3. Bot challenge.

#### Scenario: Donation initiates a Stripe session

- **WHEN** a valid donation request arrives with a verified bot-challenge token
- **THEN** the system SHALL initiate a Stripe Checkout session (or equivalent) and return the redirect URL to the caller

#### Scenario: Webhook completes the donation record

- **WHEN** Stripe later POSTs a successful payment webhook
- **THEN** the donation row SHALL be marked completed via the Stripe webhook flow (see `stripe-webhook` capability)

### Requirement: Money-moving public endpoints carry rate limit + bot challenge + CORS

Money-moving public endpoints SHALL carry all three: `money_limiter` (per-IP, 10/min), bot challenge (fail-closed when configured), and CORS allowlist. Order: `money_limiter` runs FIRST so a bursting IP cannot burn through the bot-challenge provider's quota.

#### Scenario: Rate limit precedes bot challenge

- **WHEN** an IP at the money-limiter budget submits another donation
- **THEN** the handler SHALL return 429 WITHOUT calling the bot-challenge provider

#### Scenario: A new money-moving public endpoint inherits all three gates

- **WHEN** a new endpoint is added under `/public/*` that initiates payment
- **THEN** it SHALL be wired into the bot-challenge layer, the `money_limiter`, and the CORS allowlist — all three are required
