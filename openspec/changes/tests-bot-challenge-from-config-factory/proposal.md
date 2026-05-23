## Why

`bot_challenge::from_config` in `src/api/middleware/bot_challenge.rs:217-243` is the factory that picks which `BotChallengeVerifier` implementation runs in production. It has four branches, and **none of them are tested**:

1. `provider = "disabled"` → returns `DisabledVerifier`.
2. `provider = "turnstile"` (or `"hcaptcha"`) with empty `secret_key` → emits a startup warning and **falls back to `DisabledVerifier`**.
3. `provider = "turnstile"` (or `"hcaptcha"`) with a non-empty `secret_key` → returns a real `TurnstileVerifier`.
4. Unknown provider string (typo, future provider name) → emits a startup warning and **falls back to `DisabledVerifier`**.

The fail-closed behavior of branches 2 and 4 is load-bearing for the `bot-challenge` capability spec ("Disabled is the only configuration that bypasses verification" — `openspec/specs/bot-challenge/spec.md`). A regression that flipped either of those branches the wrong way would either crash the boot (no verifier returned) or — worse — silently fall through to `TurnstileVerifier::new` with an empty secret, which would then reject every real request as `ProviderUnreachable` and look like a Cloudflare outage in production.

Behaviour can be observed without making any real HTTP call: `DisabledVerifier::verify(None, ...)` returns `Ok(())`, while `TurnstileVerifier::verify(None, ...)` returns `Err(VerifyError::Missing)` before any I/O happens. That difference is the test oracle.

`tests/bot_challenge_test.rs` already exercises `DisabledVerifier` and the in-process `FakeVerifier` directly but never goes through `from_config`. Adding the factory tests there keeps related verifier tests in one file.

## What Changes

Extend `tests/bot_challenge_test.rs` with four new `#[tokio::test]` cases that build a `BotChallengeConfig`, pass it to `from_config`, and then call `verify(None, ...)` on the returned `Arc<dyn BotChallengeVerifier>` to distinguish the disabled vs. turnstile verdict (`Ok(())` vs. `Err(VerifyError::Missing)`).

## Impact

- Tests added in `tests/bot_challenge_test.rs` (extends the existing integration test file).
- May require `coterie::api::middleware::bot_challenge::from_config` to be exported (already is — `pub fn from_config`).
- No production code changes.
- No new dependencies; `reqwest::Client::new()` is already a transitive dep through the main crate.

## Capabilities

### Modified Capabilities
- `bot-challenge`: tests lock in the fail-closed-on-misconfig invariant that the existing spec describes ("Disabled is the only configuration that bypasses verification"). The new scenarios are added to `openspec/specs/bot-challenge/spec.md`.
