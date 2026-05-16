## ADDED Requirements

### Requirement: API handlers extract granular state, not AppState

Every handler in `src/api/handlers/` SHALL extract its dependencies via `State<Arc<dyn TargetService>>` (or analogous granular wrappers, including the `LoginLimiter` / `MoneyLimiter` newtypes for rate limiters) rather than `State<AppState>`. The handler signature SHALL list exactly the services, repositories, and infrastructure components the body uses.

Exceptions are allowed for handlers with broad cross-cutting needs (≥6 extractors or otherwise unwieldy). Such exceptions SHALL carry a brief inline comment explaining why. The default position is granular extraction.

Middleware in `src/api/middleware/` SHALL continue to use `State<AppState>` — `FromRef` is for handler extraction, not for middleware wired via `from_fn_with_state`.

#### Scenario: API handler signature names its dependencies

- **WHEN** a reader inspects any handler in `src/api/handlers/`
- **THEN** the granular extractors SHALL enumerate exactly the dependencies the body uses; no domain navigation needed

#### Scenario: Webhook handler exception is documented

- **WHEN** `stripe_webhook` (or any other handler with genuine cross-cutting needs) retains `State<AppState>`
- **THEN** a brief inline comment SHALL explain the choice; bare `State<AppState>` without justification SHALL be treated as a defect

#### Scenario: Login limiter consumed via its newtype

- **WHEN** the `login` handler extracts the rate limiter
- **THEN** it SHALL extract `State<LoginLimiter>` (the newtype introduced by `add-fromref-impls-on-appstate`), not `State<RateLimiter>` (which doesn't exist as an unambiguous extractor)
