## MODIFIED Requirements

### Requirement: Disabled provider is an explicit opt-out

The system SHALL accept `bot_challenge.provider = "disabled"` as an explicit opt-out for local development or orgs that have not configured a provider. The disabled verifier SHALL ignore the token, return success, and emit a debug-level trace so the bypass is visible at elevated log levels.

#### Scenario: Disabled provider lets unsigned requests through

- **WHEN** `bot_challenge.provider` is `"disabled"` and a request reaches a public endpoint
- **THEN** the bot-challenge layer SHALL pass the request through and emit a debug-level trace

#### Scenario: Disabled is the only configuration that bypasses verification

- **WHEN** `bot_challenge.provider` is set to anything other than `"disabled"`
- **THEN** verification SHALL run and the request SHALL fail closed on any error

#### Scenario: Misconfigured non-disabled provider falls back to the disabled verifier

- **WHEN** the provider name is `"turnstile"` or `"hcaptcha"` but `secret_key` is empty, OR the provider name is not recognized at all
- **THEN** `from_config` SHALL return the disabled verifier (so a misconfigured deployment does not silently treat every request as `ProviderUnreachable`) AND emit a startup-level warning so an operator notices

#### Scenario: Well-configured turnstile provider returns the real verifier

- **WHEN** the provider name is `"turnstile"` or `"hcaptcha"` AND `secret_key` is non-empty
- **THEN** `from_config` SHALL return a verifier whose `verify(None, ...)` returns `Err(VerifyError::Missing)` — i.e. a real `TurnstileVerifier`, not the disabled fallback
