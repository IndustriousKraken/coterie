## Context

`src/web/portal/admin/members/mod.rs` is 1090 lines of admin-member handler functions. The file already has a `// =====` divider around line 453 between the "member CRUD" section and "payment recording" section — the author was already conceptually organizing it, just hadn't extracted submodules yet.

The pattern of splitting an admin area into per-concern files is established elsewhere in the codebase (`admin/events/`, `admin/announcements/`). This change applies the same pattern here.

## Goals / Non-Goals

**Goals:**
- Each submodule ≤300 lines including any local helpers.
- Routing centralized in `mod.rs`.
- Each submodule's handlers are coherent — a reader looking for "how does dues admin work?" opens `dues.rs` and sees everything.

**Non-Goals:**
- Changing handler behavior, signatures, or routes.
- Refactoring the Axum extractors/responses.
- Touching `admin/events/`, `admin/announcements/`, or other admin areas (those are separate concerns and not part of the architecture finding).

## Decisions

### D1. mod.rs holds the router + module declarations only

`mod.rs` after the split is small: `mod list; mod detail; ...` declarations, the `pub fn routes()` function (or whichever function returns the `Router`) that wires up paths to handlers, and any shared imports.

### D2. Handlers are `pub` (or `pub(super)`)

Axum handlers need to be referenceable from the router. The simplest pattern: each handler is `pub async fn ...` and the router in `mod.rs` references them as `list::admin_members_page`, `detail::admin_member_detail_page`, etc.

If `pub(super)` is enough (handlers only used by the immediate parent's router), prefer that — it keeps the visibility tight. The autocoder should use the narrowest visibility that compiles.

### D3. Local helpers stay with their handlers

`parse_dollars_to_cents`, `rerender_with_error`, `discord_id_result`, `resend_result` are private helpers used only by specific handlers. They stay in the same submodule as their callers (private, not `pub`).

### D4. Tests aren't in this file

Unlike `a27`'s member_service split, this file has no inline tests. Integration tests for these handlers live in `tests/` (router-level tests). Nothing to relocate.

## Risks / Trade-offs

- **Risk**: an Axum route reference becomes wrong after the split. → Mitigation: `cargo build` catches this; integration tests confirm routes still resolve.
- **Risk**: subtle visibility issues — a handler that was implicitly accessible inside the same file is no longer accessible across files. → Mitigation: use `pub(super)` where the consumer is the parent module; otherwise `pub`. Cargo will surface any access issues.
- **Trade-off**: 8+ small files instead of one big one. The codebase already does this for other admin areas; the consistency is worth it.

## Migration Plan

Single PR.

1. Create the submodule files: `list.rs`, `detail.rs`, `create.rs`, `status.rs`, `dues.rs`, `payments.rs`, `discord.rs`, `verification.rs`.
2. Move each function group to its target submodule, including any private helpers it depends on.
3. Reconcile imports — each submodule needs its own `use` block.
4. Update `mod.rs` to declare all the submodules and reference handlers from the new paths in its router.
5. `cargo build` clean, `cargo test --features test-utils` passes.
6. `cargo clippy --features test-utils -- --deny warnings` clean.
