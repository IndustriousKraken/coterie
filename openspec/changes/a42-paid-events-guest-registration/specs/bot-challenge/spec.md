# bot-challenge Specification

## MODIFIED Requirements

### Requirement: Public state-changing endpoints require a verified bot-challenge token

`POST /public/signup`, `POST /public/donate`, and `POST /public/events/:id/register` SHALL require a Turnstile-compatible bot-challenge token in the request body. The system SHALL verify the token with the configured provider before invoking the underlying handler.

Public paid-event registration is included for the same reason as the other two: it is an unauthenticated endpoint that initiates a Stripe charge, which is exactly the exposure that produced card-testing abuse against the existing public endpoints. Any future unauthenticated endpoint that initiates a charge SHALL be added to this list as part of the change that introduces it.

#### Scenario: Valid token is accepted

- **WHEN** a `/public/signup` request includes a token the provider verifies as valid
- **THEN** the handler SHALL be invoked and the structured log SHALL record `outcome = "ok"`

#### Scenario: Missing token fails closed

- **WHEN** the bot-challenge provider is configured (not `disabled`) and a request omits the token
- **THEN** the request SHALL be rejected with 403 Forbidden and the log SHALL record `outcome = "missing"`

#### Scenario: Provider rejects token

- **WHEN** the provider returns `success: false`
- **THEN** the request SHALL be rejected with 403 and the provider's error codes SHALL be logged for observability

#### Scenario: Provider unreachable fails closed

- **WHEN** the provider does not respond within the configured timeout, or returns a non-2xx status
- **THEN** the request SHALL be rejected with 403 and the log SHALL record `outcome = "provider_unreachable"`

#### Scenario: Public event registration fails closed without a token

- **WHEN** the provider is configured and a `POST /public/events/:id/register` request omits the token
- **THEN** the request SHALL be rejected with 403 and no seat, payment row, or Checkout session SHALL be created
