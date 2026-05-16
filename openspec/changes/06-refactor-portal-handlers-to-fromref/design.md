## Context

After `add-fromref-impls-on-appstate` lands, `State<Arc<dyn TargetService>>` extraction is available everywhere. This change migrates portal-side handlers to use it.

Mapping the work surface (verified via grep against the pre-change codebase):

- `src/web/templates/` — 4 files, pre-auth flows.
- `src/web/portal/` (member-facing, non-admin) — 8 files.
- `src/web/portal/admin/` — 11 files; this is the bulk.

~90 handler signatures total across the portal side. Each migration:

1. Look at the handler body, determine which `state.<path>` references it actually uses.
2. Replace `State(state): State<AppState>` with one or more granular `State(<name>): State<Arc<<Type>>>` extractors.
3. Rewrite the body references: `state.service_context.member_repo.find_by_id(...)` becomes `member_repo.find_by_id(...)`.

The work is mechanical but voluminous. The autocoder's per-change budget bounds the safe scope; we run with the area-organized task list below and escalate to a further split if it's still too much.

## Goals / Non-Goals

**Goals:**
- Every handler in `src/web/templates/` and `src/web/portal/` (including `admin/`) extracts only the dependencies it actually uses.
- Handler signatures self-document their dependencies.
- Wire shape unchanged.

**Non-Goals:**
- Refactoring API handlers (`src/api/handlers/`). That's `refactor-api-handlers-to-fromref`.
- Refactoring middleware. Middleware via `from_fn_with_state` is passed `AppState` explicitly; FromRef is for `Router::with_state`-extracted handler state. Middleware stays on `State<AppState>`.
- Changing `BaseContext` or other helper signatures unless required to avoid `State<AppState>` retention. See D2.
- Removing `State<AppState>` from a handler that legitimately needs many components and would otherwise have an unwieldy signature. See D3.

## Decisions

### D1. Migration is per-handler, not per-file

A handler that uses two repos and one service gets two `State<Arc<dyn Repo>>` extractors and one `State<Arc<Svc>>`. Multiple handlers in the same file don't share extractors — each handler declares its own. Verbose, but each handler's dependencies are visible at its own signature.

### D2. `BaseContext::for_member` should be refactored to take granular inputs

Today the helper takes `(&AppState, &CurrentUser, &SessionInfo)` and internally reads `state.service_context.csrf_service` and the `current_user`. Refactor to take `(csrf_service: &CsrfService, current_user: &CurrentUser, session: &SessionInfo)`. Handlers that build a `BaseContext` then extract `State<Arc<CsrfService>>` granularly, deref to `&CsrfService`, and pass it.

This decision saves the largest "I need many things just to build a BaseContext" pressure on handler signatures. Without it, every page handler would retain `State<AppState>` just for the helper.

### D3. Allow `State<AppState>` retention as a documented exception

A handler that genuinely uses 6+ services or has cross-cutting needs MAY retain `State<AppState>`. The standard is "if a meaningful subset is identifiable, extract granularly." Examples likely to be exceptions:

- Handlers that construct several different integration events plus call multiple services (some admin actions).
- Handlers that need access to `Settings` for many fields plus repos.

The exception is opt-in. The default is granular extraction. When the autocoder lands a handler with `State<AppState>` retained, the surrounding comment should briefly explain why.

### D4. Body-rewrite mechanics

Where the original handler had:

```rust
let member = state.service_context.member_repo.find_by_id(id).await?;
state.service_context.audit_service.log(...).await;
```

…the migrated handler has:

```rust
let member = member_repo.find_by_id(id).await?;
audit.log(...).await;
```

…where `member_repo` and `audit` are the extracted names from the signature. Variable names in the signature should match how the body refers to them.

### D5. Extension extractors (CurrentUser, SessionInfo) stay

`Extension(current_user): Extension<CurrentUser>` and `Extension(session_info): Extension<SessionInfo>` are independent of the state-extraction question. They keep working exactly as today; no change.

### D6. Routers don't move

`src/web/portal/mod.rs` keeps registering routes against `AppState` (`.with_state(state)`). Axum's `FromRef` machinery is what makes granular handler extraction work against a `Router<AppState>`. No router file changes.

### D7. Pre-auth handlers (`src/web/templates/`) are simpler

Login, setup, password reset, verify — these handlers typically need 1–3 services (`AuthService`, `LoginLimiter`, `MemberRepository`, `EmailTokenService`). Easy migrations; do these first to warm up the pattern.

### D8. Admin-portal handlers are the heaviest area

The 11 files in `src/web/portal/admin/` contain the most handlers and the deepest service-call chains. `members.rs` alone (after `lift-member-admin-orchestration`) has its handler bodies thinned out — they mostly delegate to `MemberService` — but the chain still includes audit, integration, and renderers. Most admin handlers will need 2–4 granular extractors.

### D9. Member-facing portal handlers are mid-weight

`dashboard.rs`, `profile.rs`, `events.rs`, `announcements.rs`, `payments.rs`, etc. typically need a few repos plus the membership-type service plus a CSRF service for `BaseContext`. Mid-weight migrations.

## Risks / Trade-offs

- **Risk**: a handler is migrated but its body misses a `state.<path>` reference that no longer resolves. → **Mitigation**: `cargo build` catches every miss; the type system is the regression net.
- **Risk**: signatures become unwieldy for handlers that need many things. → **Mitigation**: D3 explicitly allows `State<AppState>` retention as an exception with a comment.
- **Trade-off**: per-handler verbose extractors vs. one `State<AppState>` line. The verbosity *is* the value — dependencies are visible at the signature.
- **Trade-off**: this change touches a lot of files in one PR. Autocoder may still time out. If so, escalate to splitting along `admin/` vs. member-facing along the lines the task structure already implies.

## Migration Plan

Single PR (escalate to two if needed). The task order below intentionally goes simplest-area first so partial progress is well-defined.

1. Refactor `BaseContext::for_member` to take granular inputs (D2).
2. Pre-auth handlers (`src/web/templates/`) — simplest area, smallest count.
3. Member-facing portal handlers (`src/web/portal/{dashboard,profile,security,events,announcements,payments,donations,restore}.rs`).
4. Admin portal handlers (`src/web/portal/admin/*.rs`) — heaviest area, last.
5. `cargo build --all-targets --features test-utils`.
6. `cargo test --features test-utils`.
