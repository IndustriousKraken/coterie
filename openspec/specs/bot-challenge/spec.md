# bot-challenge Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Public state-changing endpoints require a verified bot-challenge token

`POST /public/signup` and `POST /public/donate` SHALL require a Turnstile-compatible bot-challenge token in the request body. The system SHALL verify the token with the configured provider before invoking the underlying handler.

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

### Requirement: Disabled provider is an explicit opt-out

The system SHALL accept `bot_challenge.provider = "disabled"` as an explicit opt-out for local development or orgs that have not configured a provider. The disabled verifier SHALL ignore the token, return success, and emit a debug-level trace so the bypass is visible at elevated log levels.

#### Scenario: Disabled provider lets unsigned requests through

- **WHEN** `bot_challenge.provider` is `"disabled"` and a request reaches a public endpoint
- **THEN** the bot-challenge layer SHALL pass the request through and emit a debug-level trace

#### Scenario: Disabled is the only configuration that bypasses verification

- **WHEN** `bot_challenge.provider` is set to anything other than `"disabled"`
- **THEN** verification SHALL run and the request SHALL fail closed on any error

### Requirement: Verifier is swappable via trait

The system SHALL abstract bot-challenge verification behind a `BotChallengeVerifier` trait so tests and alternative providers can substitute the implementation without an HTTP mock.

#### Scenario: Test substitutes a fake verifier

- **WHEN** integration tests want to control verification outcome
- **THEN** they SHALL inject a fake `BotChallengeVerifier` implementation rather than mocking the network

