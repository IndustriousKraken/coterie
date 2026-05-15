## ADDED Requirements

### Requirement: Gating middlewares share a single core implementation

The four gating middlewares (`require_auth`, `require_auth_redirect`, `require_restorable`, `require_admin_redirect`) and `optional_auth` SHALL share a single `authenticate(...)` core that performs the cookie-read, session-validate, member-load, and status-check sequence exactly once. Per-variant differences (allowed status set, on-reject behavior, admin-flag check, TOTP enforcement) SHALL be expressed as data — typically an `AccessPolicy` value — passed into the shared core, not as duplicated imperative code in each middleware body.

A future change to session validation, member loading, or `CurrentUser`/`SessionInfo` injection SHALL only need to land in the shared core; per-variant wrapper code SHALL NOT need to be touched.

#### Scenario: New variant uses the shared core

- **WHEN** a contributor adds a new auth middleware variant (e.g., a hypothetical `require_member_or_admin`)
- **THEN** the variant SHALL be expressed as a small wrapper that builds an `AccessPolicy` and delegates to the shared `authenticate(...)`; it SHALL NOT re-implement cookie/session/member-loading logic

#### Scenario: Session validation fix lands in one place

- **WHEN** a contributor changes how `auth_service.validate_session(...)` is called (e.g., adds session-id rotation)
- **THEN** the change SHALL land inside the shared core function and SHALL automatically apply to every wrapper without per-wrapper edits

### Requirement: Middlewares use the shared MemberRepository from ServiceContext

Auth middleware SHALL load members via `state.service_context.member_repo` (the shared `Arc<dyn MemberRepository>`). Constructing a fresh `SqliteMemberRepository::new(...)` inside the middleware body SHALL NOT be done.

#### Scenario: Member-load uses the shared repo Arc

- **WHEN** any auth middleware loads the current member
- **THEN** it SHALL call `state.service_context.member_repo.find_by_id(...)`; it SHALL NOT call `SqliteMemberRepository::new(state.service_context.db_pool.clone())`

#### Scenario: Tests can inject a fake member repo

- **WHEN** a test wires a fake `MemberRepository` into `ServiceContext`
- **THEN** the auth middleware SHALL exercise the fake (it cannot bypass it by constructing its own SQLite repo)

### Requirement: Public middleware names and signatures are stable

`require_auth`, `require_auth_redirect`, `require_restorable`, `require_admin_redirect`, and `optional_auth` SHALL remain the public symbols routers reference. Their signatures SHALL match the existing `(State<AppState>, CookieJar, Request, Next) -> Response` (or `Result<Response, AppError>` for `require_auth`) shape so no router file is forced to change as a consequence of the internal consolidation.

#### Scenario: Routers do not move when the internals consolidate

- **WHEN** the auth-middleware refactor ships
- **THEN** `src/api/mod.rs`, `src/web/portal/mod.rs`, and `src/web/mod.rs` SHALL NOT need import or signature changes; the same `route_layer(from_fn_with_state(state, require_*))` lines continue to compile
