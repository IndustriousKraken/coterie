## Context

This change mirrors `refactor-portal-handlers-to-fromref` but applies to the smaller API surface under `src/api/handlers/`. Per CLAUDE.md, the JSON surface is intentionally narrow: Stripe webhook, saved-card endpoints (after `narrow-saved-card-json-surface`, only `setup-intent` and `POST /cards`), public marketing-site endpoints (signup, donate, feeds), and auth (login/logout). About 4 files, ~10–15 handlers in total.

Each handler typically needs 2–5 dependencies. Migrations are mechanical: replace `State(state): State<AppState>` with granular extractors, rewrite body references.

## Goals / Non-Goals

**Goals:**
- Every API handler extracts only the dependencies it uses.
- Wire shape unchanged.
- Handler signatures self-document their dependencies.

**Non-Goals:**
- Refactoring portal handlers (covered by `refactor-portal-handlers-to-fromref`).
- Refactoring middleware (`src/api/middleware/*.rs`). Middleware via `from_fn_with_state` keeps `State<AppState>` since `FromRef` is for handler extraction.
- Changing the API surface itself (CLAUDE.md says it stays narrow).
- Changing OpenAPI documentation (`src/api/docs.rs`). It documents `pub async fn` signatures and uses `#[utoipa::path(...)]` — changing the `State<>` extractor doesn't affect the OpenAPI doc since utoipa reads request/response types, not state extraction.

## Decisions

### D1. Same migration mechanics as the portal change

Per-handler granular extraction. Body rewrites match what the signature extracts. Variable names match between signature and body. See `refactor-portal-handlers-to-fromref/design.md` for the canonical pattern.

### D2. `Option<Arc<StripeClient>>` and `Option<Arc<WebhookDispatcher>>` extract as Options

The `signup` handler doesn't need Stripe at all. The `donate` handler needs it. The `stripe_webhook` handler needs the dispatcher. Each handler extracts the optional Arc and either matches on `None` to reject (Stripe not configured) or proceeds with the configured value. Same pattern as today.

### D3. Webhook handler is the heaviest

`stripe_webhook` in `payments.rs` is the lone receiver of inbound Stripe events. It reaches into many services: `webhook_dispatcher` (which itself holds repos), `billing_service` for dunning, `payment_repo` for the body, etc. After the migration it'll have ~4 granular extractors. If the signature gets uncomfortable, D3 in the portal-handlers design applies: retain `State<AppState>` with a comment.

### D4. Root/health handlers stay simple

`root::root`, `root::health_check`, `root::api_info` may not extract state at all today. Check during migration — if they take `State<…>`, remove it; if they don't, leave them alone.

### D5. Login/logout audit emission

The current `login` and `logout` handlers in `src/api/handlers/auth.rs` call `audit_service.log` directly (per the existing `audit-logging` spec, "Logout writes a session audit row" is emitted from the handler). The migration preserves that — just extract `State<Arc<AuditService>>` granularly and call it.

### D6. `LoginLimiter` newtype is consumed here

The `add-fromref-impls-on-appstate` change introduced `LoginLimiter` / `MoneyLimiter` newtypes (the unwrap-to-`RateLimiter` extractor pattern). The `login` handler is the canonical consumer; it extracts `State<LoginLimiter>` and uses its inner `RateLimiter` via the newtype's deref or `.0` access.

## Risks / Trade-offs

- **Risk**: stripe_webhook extraction misses a dependency that becomes obvious only at runtime. → **Mitigation**: the existing webhook tests (`tests/stripe_webhook_test.rs`) cover the major event types; they're the regression net.
- **Trade-off**: Webhook handler signature is the largest in this change. Acceptable; the alternative (retain `State<AppState>` for one handler) is explicitly allowed per D3.

## Migration Plan

Single PR.

1. `src/api/handlers/root.rs` — quickest, possibly no migrations needed.
2. `src/api/handlers/announcements.rs` — single small handler.
3. `src/api/handlers/auth.rs` — login + logout.
4. `src/api/handlers/public.rs` — signup, donate, feeds.
5. `src/api/handlers/payments.rs` — stripe_webhook, create_setup_intent, save_card. Heaviest.
6. `cargo build --all-targets --features test-utils`.
7. `cargo test --features test-utils`.
