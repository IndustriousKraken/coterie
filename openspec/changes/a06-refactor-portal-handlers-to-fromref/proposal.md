## Why

After `add-fromref-impls-on-appstate` lands, `AppState` exposes `FromRef<AppState>` impls for every service, repository, and piece of infrastructure a handler might want. This change uses those impls — it migrates all portal-side and pre-auth-page handlers from `State(state): State<AppState>` extraction to granular `State(svc): State<Arc<dyn TargetService>>` extraction, one specific dependency at a time.

The motivation is principle of least privilege at the handler signature. Today a reader of any portal handler sees `State<AppState>` and has to read the body to find out what the handler actually uses. After this change, the signature itself documents the dependencies — `State(member_repo): State<Arc<dyn MemberRepository>>, State(audit): State<Arc<AuditService>>` says exactly what a handler reaches for.

This is the second of three changes split out from the original `refactor-from-ref-state` work. It covers `src/web/templates/*.rs` and `src/web/portal/**/*.rs` — about 17 files containing ~90 handler signatures. The change is bounded by area so an autocoder run completes within the per-change budget; if the portal-admin subtree alone exhausts that budget, a follow-up split into `06a` (admin) and `06b` (member-facing) is the natural escalation.

## What Changes

- **For every handler in `src/web/templates/`** (`auth.rs`, `setup.rs`, `reset.rs`, `verify.rs`): replace `State<AppState>` with one or more granular `State<Arc<…>>` extractors based on what the handler body actually uses. Handler body code that read `state.<path>` becomes a direct reference to the extracted Arc.
- **For every handler in `src/web/portal/`** (member-facing): same migration. Files: `dashboard.rs`, `profile.rs`, `security.rs`, `events.rs`, `announcements.rs`, `payments.rs`, `donations.rs`, `restore.rs`, plus `partials.rs` if it contains handlers.
- **For every handler in `src/web/portal/admin/`**: same migration. Files: `members.rs`, `events.rs`, `announcements.rs`, `types.rs`, `settings.rs`, `email.rs`, `discord.rs`, `billing.rs`, `audit.rs`, plus `partials.rs` if it contains handlers.
- **Handlers using `BaseContext::for_member(&state, ...)`**: this helper needs the full `AppState` (or its `csrf_service` + `current_user`). Two options at the migration site:
  - Pass the few needed parts to a new `BaseContext::for_member(csrf_service, current_user, session)` signature (preferred — it shrinks the helper's surface), OR
  - Keep `State<AppState>` on handlers that build a `BaseContext`. Acceptable as a transitional measure; flagged in design.md.
- **Handlers that legitimately need many dependencies** (e.g., the member-admin handlers that call MemberService AND audit AND integration AND email): extract the smallest sufficient set. If a handler genuinely needs ≥4 things and the signature gets unwieldy, the design notes a fallback (group into a cohesive aggregate via a small `HandlerCtx` extractor type).
- **No behavioral changes.** URLs, response shapes, status codes, redirect targets, audit row contents, integration events — all unchanged.
- **No routing changes.** Router files (`src/api/mod.rs`, `src/web/portal/mod.rs`, `src/web/mod.rs`) keep passing `AppState` via `.with_state(state)` and `.layer(from_fn_with_state(state, …))`. Only handler signatures change.

## Capabilities

### New Capabilities

(None — this is an internal refactor.)

### Modified Capabilities
- `routing-architecture`: tightens the handler-style requirement. Portal handlers SHALL extract granular state via `State<Arc<…>>` rather than `State<AppState>` (with documented exceptions where genuine cross-cutting dependencies justify the broader extractor — e.g., a handler that constructs a `BaseContext` if the helper isn't itself refactored).

## Impact

- **Code**: ~17 files in `src/web/templates/` and `src/web/portal/` (including `admin/`), ~90 handler signatures rewritten. Bodies change only where they previously read `state.<path>` — those references become the extracted variable name. Mechanical scope, but voluminous.
- **Wire shape**: zero change.
- **Tests**: existing handler-level tests assert HTTP responses; they continue to pass. `cargo build` catches any incomplete migration.
- **Risk**: medium. The volume of mechanical edits invites typos. Mitigation: per-file commits during the autocoder run so partial progress is preserved if a timeout occurs; the compiler catches any signature/body mismatch.
- **Dependency**: this change requires `add-fromref-impls-on-appstate` to have landed.
- **Sequencing**: `refactor-api-handlers-to-fromref` is independent of this change. Both can run in either order after `05` lands.
- **Escalation**: if this single change still exhausts the autocoder budget, the natural further split is `06a-refactor-portal-admin-handlers` (the heaviest area — ~50 handlers across 11 files) and `06b-refactor-portal-member-handlers-to-fromref`. The task structure below is organized by area to make that further split easy if needed.
