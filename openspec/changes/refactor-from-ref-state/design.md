## Context

In an Axum application, the `State` extractor allows handlers to retrieve dependencies. Currently, Coterie passes a single `AppState` God Object to all handlers. This `AppState` contains a `ServiceContext` which holds `Arc`s to all repositories and services, along with `StripeClient`, `BillingService`, rate limiters, etc. Every handler has access to the entire application domain, which breaks the principle of least privilege, complicates testing, and obscures the specific dependencies required by any given route.

## Goals / Non-Goals

**Goals:**
- Implement `axum::extract::FromRef` for individual services, repositories, and limiters within `AppState`.
- Refactor all existing Axum handlers (in `src/api/` and `src/web/`) to extract only the state they actually need using `State(svc): State<Arc<dyn TargetService>>`.
- Make handler signatures self-documenting regarding their dependencies.

**Non-Goals:**
- Completely decoupling `AppState` at the router initialization level (the root router will still hold `AppState` and Axum will use `FromRef` internally during extraction).
- Changing business logic or repository implementations.
- Refactoring `ServiceContext`'s internal structure at this time (it will just be traversed by `FromRef`).

## Decisions

1. **Implement `FromRef<AppState>`:**
   We will write `FromRef` implementations for all `Arc`s currently accessed by handlers. E.g.:
   ```rust
   impl axum::extract::FromRef<AppState> for Arc<dyn MemberRepository> {
       fn from_ref(state: &AppState) -> Self {
           state.service_context.member_repo.clone()
       }
   }
   // Repeat for other services/limiters.
   ```
   
2. **Update Handler Signatures:**
   A handler currently defined as:
   ```rust
   pub async fn get_member(State(state): State<AppState>, ...)
   ```
   Will be updated to:
   ```rust
   pub async fn get_member(State(member_repo): State<Arc<dyn MemberRepository>>, ...)
   ```
   If a handler needs multiple dependencies, it will extract multiple specific states or, if that becomes unwieldy, we may group a few cohesive dependencies.

3. **Rate Limiters and Locks:**
   Components like `login_limiter` and `setup_lock` will also have `FromRef` implemented if handlers need direct access to them.

## Risks / Trade-offs

- **Verbose Router Implementations:** Axum's `FromRef` requires explicit boilerplate for every type we want to extract. We will mitigate this by implementing a macro if the boilerplate becomes excessive, though manual implementation is fine for ~20-30 services.
- **Merge conflicts:** This is a wide-reaching change touching every handler. It should be merged when other feature work is light.