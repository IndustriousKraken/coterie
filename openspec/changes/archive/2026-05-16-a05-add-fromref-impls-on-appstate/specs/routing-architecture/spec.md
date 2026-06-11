## ADDED Requirements

### Requirement: AppState exposes FromRef impls for granular extraction

`AppState` SHALL expose `axum::extract::FromRef<AppState>` implementations for every constituent service, repository, and piece of infrastructure that a handler might reasonably extract. Adding a new field to `AppState` or `ServiceContext` SHALL also include a `FromRef<AppState>` impl in the same change.

The impls SHALL all live in `src/api/state.rs` (or a clearly-scoped sub-module of it) so that "what `AppState` exposes" can be answered by reading a single file.

Handlers MAY continue to use `State<AppState>` extraction; this requirement does not mandate a particular handler style. It only requires that the FromRef machinery is available so granular extraction is possible.

#### Scenario: Every field is extractable

- **WHEN** a handler writes `State(svc): State<Arc<dyn SomeRepository>>` against a router holding `AppState`
- **THEN** the extraction SHALL resolve via `FromRef<AppState>` to `state.service_context.<field>.clone()` (or the analogous path for non-`service_context` fields)

#### Scenario: A new field on AppState gets a FromRef impl

- **WHEN** a contributor adds a new service, repo, or infrastructure component to `AppState` (or to `ServiceContext` reachable through `AppState`)
- **THEN** the same change SHALL include a `FromRef<AppState>` impl for it in `src/api/state.rs`

#### Scenario: Existing State<AppState> handlers still compile

- **WHEN** a handler authored before this change uses `State(state): State<AppState>`
- **THEN** that handler SHALL continue to compile and run unchanged — the FromRef impls coexist with the old extraction shape

### Requirement: Distinct RateLimiter instances are extractable via newtypes

The two `RateLimiter` instances on `AppState` (`login_limiter` and `money_limiter`) SHALL each be wrapped in a newtype (`LoginLimiter`, `MoneyLimiter`) so they can be disambiguated as `State<LoginLimiter>` vs. `State<MoneyLimiter>` extractors. A bare `FromRef<AppState> for RateLimiter` SHALL NOT exist (it would be ambiguous between the two instances).

#### Scenario: Login limiter extracts via its newtype

- **WHEN** a handler writes `State(limiter): State<LoginLimiter>`
- **THEN** the extraction SHALL resolve to a clone of `state.login_limiter` wrapped in the `LoginLimiter` newtype

#### Scenario: Money limiter extracts via its newtype

- **WHEN** a handler writes `State(limiter): State<MoneyLimiter>`
- **THEN** the extraction SHALL resolve to a clone of `state.money_limiter` wrapped in the `MoneyLimiter` newtype
