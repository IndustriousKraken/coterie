## Context

`src/api/state.rs` defines `AppState` as a struct with public fields holding every service, repository, and piece of infrastructure the application needs. Today handlers take `State(state): State<AppState>` and then reach in via `state.service_context.member_repo` etc. Axum supports a more granular pattern via `axum::extract::FromRef`: any `impl FromRef<AppState> for T` enables handlers to write `State(t): State<T>`.

The original `refactor-from-ref-state` change bundled adding the impls with refactoring every handler to use them — 19 tasks total, but the handler-refactor sections covered ~90 handler signatures in 17 portal files plus another ~10 in api handlers. That blew through the autocoder's 30-minute budget. Splitting addresses both the budget and the natural seam in the work: the impls are pure additions, the handler refactors are mechanical sweeps.

This change is the first of three. It adds only the impls. Handlers stay on `State<AppState>` until the follow-up changes migrate them.

## Goals / Non-Goals

**Goals:**
- Every component on `AppState` that a handler might want is reachable via `FromRef<AppState>`.
- No handler is modified.
- The codebase compiles cleanly after this change with no new warnings.

**Non-Goals:**
- Modifying handler signatures (deferred to `refactor-portal-handlers-to-fromref` and `refactor-api-handlers-to-fromref`).
- Removing the `State<AppState>` pattern from any handler.
- Restructuring `AppState`, `ServiceContext`, or any field's type.
- Introducing macros to reduce the impl boilerplate. The original design considered this — leave it for now; 40–60 impls is repetitive but readable, and a macro can be added later if the boilerplate becomes a maintenance burden.
- Adding `FromRef` impls for types that are not held by `AppState`. If a handler later wants a derived type (e.g., a tuple of two services), it can extract them separately.

## Decisions

### D1. One impl per field, located in `src/api/state.rs`

Every impl lives in `src/api/state.rs` immediately after the `AppState` struct definition. Keeping them co-located means a reader can see "what `AppState` is" and "what's extractable from it" in one file.

### D2. Impl bodies are uniform clones

```rust
impl axum::extract::FromRef<AppState> for Arc<dyn MemberRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.member_repo.clone()
    }
}
```

Every impl follows this shape: walk to the field on `AppState` or `ServiceContext`, clone the `Arc`. `Arc::clone` is cheap (atomic refcount bump).

### D3. Cover every plausibly-extractable field

The proposal lists ~30 targets. Better to author all the impls now than to do another sweep later when a handler needs an extractor that doesn't exist yet. The impls are cheap (3–5 lines each); the alternative ("only add what's needed") forces a second round-trip every time a new extraction site appears.

### D4. `RateLimiter` extraction needs disambiguation

`AppState` holds two `RateLimiter` instances (`login_limiter`, `money_limiter`). A bare `impl FromRef<AppState> for RateLimiter` would be ambiguous. Two options:

- **Option A**: introduce newtypes (`pub struct LoginLimiter(pub RateLimiter);` and `pub struct MoneyLimiter(pub RateLimiter);`) and provide `FromRef` for each.
- **Option B**: don't add `FromRef` for limiters; the (few) handlers that need them continue to extract `State<AppState>` for now.

Pick **Option A**. The newtype is a 3-line declaration and makes the limiter intent visible at the call site (`State(login_limiter): State<LoginLimiter>` reads better than two limiters indistinguishable by type).

### D5. `Option<Arc<StripeClient>>` and `Option<Arc<WebhookDispatcher>>` are extractable as `Option<…>`

Stripe configuration is optional. Handlers that need Stripe today match on `state.stripe_client.as_ref()`; the FromRef pattern carries the same `Option`. `impl FromRef<AppState> for Option<Arc<StripeClient>>` clones the optional Arc. Handlers continue to handle the `None` case.

### D6. No spec on which handlers SHOULD use FromRef vs. AppState

This change doesn't dictate handler style. The follow-up changes (`06`, `07`) carry the policy that handlers SHOULD prefer granular extraction. Until those land, both patterns coexist legally.

### D7. Module organization stays flat

All impls in `src/api/state.rs`. Could put them in a sub-module (`state::fromref`) for organization, but with ~30 impls all 3–5 lines each, a flat file is easier to scan than navigating a sub-module.

## Risks / Trade-offs

- **Risk**: a future field added to `AppState` ships without a `FromRef` impl. → **Mitigation**: a brief code comment at the top of the impl block notes the convention ("Every field on AppState or ServiceContext should have a FromRef impl below — see CLAUDE.md / spec"). Plus the spec delta makes the requirement explicit.
- **Trade-off**: ~30 impls of boilerplate in one file. The alternative (a macro) is more clever but harder to grep through. Boilerplate wins for now.
- **Trade-off**: this change ships ~150–200 lines of code that isn't called by anything until `06` and `07` land. The compiler is fine with unused trait impls; there are no warnings. The temporary "dead" status is acceptable as a deliberate split.

## Migration Plan

Single PR. Pure-additive.

1. Add the `FromRef<AppState>` impls in `src/api/state.rs` after the `AppState` struct.
2. Add newtype wrappers (`LoginLimiter`, `MoneyLimiter`) and their `FromRef` impls.
3. `cargo build --all-targets --features test-utils` — clean.
4. `cargo test --features test-utils` — full suite passes (no changes expected, all handlers still use `State<AppState>`).
5. Deploy normally; no flags, no migrations.

Follow-ups (`refactor-portal-handlers-to-fromref` and `refactor-api-handlers-to-fromref`) consume these impls.
