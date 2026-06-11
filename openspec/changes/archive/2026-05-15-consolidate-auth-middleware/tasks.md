## 1. Shared core types

- [x] 1.1 In `src/api/middleware/auth.rs`, add private types: `AccessPolicy { allowed_statuses: &'static [MemberStatus], require_admin: bool, enforce_admin_totp: bool, on_reject: RejectBehavior }`, `RejectBehavior` enum (`Json401`, `RedirectToLogin`, `RedirectToRestoreOrLogin`, `RedirectToDashboardOrLogin`), `Authenticated { member: Member, session_id: String }`, and `RejectReason` enum (`NoCookie`, `InvalidSession`, `MemberNotFound`, `StatusBlocked(MemberStatus)`, `NotAdmin`, `AdminTotpMissing`).
- [x] 1.2 Add the private async `authenticate(state: &AppState, jar: &CookieJar, policy: &AccessPolicy) -> Result<Authenticated, RejectReason>` core. It SHALL: read the session cookie; call `state.service_context.auth_service.validate_session(...)`; load the member via `state.service_context.member_repo.find_by_id(...)`; check `member.status` against `policy.allowed_statuses`; if `policy.require_admin`, check `member.is_admin`; if `policy.enforce_admin_totp` and the `auth.require_totp_for_admins` setting is true, check TOTP enrollment (defaulting to not-enforced on setting-lookup error to preserve the current safety semantics).
- [x] 1.3 Add a private `render_reject(reason: RejectReason, behavior: RejectBehavior, original_uri: &Uri) -> Response` helper that produces the right Response for each `(reason, behavior)` pair: `Json401` returns 401 normally and 403 specifically for `StatusBlocked(Pending)`; `RedirectToLogin` always sends to login; `RedirectToRestoreOrLogin` sends `StatusBlocked(Expired)` to `/portal/restore` and everything else to login; `RedirectToDashboardOrLogin` sends `NotAdmin` to `/portal/dashboard`, `AdminTotpMissing` to `/portal/profile/security?reason=admin_totp_required`, and everything else to login.

## 2. Migrate `require_auth`

- [x] 2.1 Rewrite `require_auth` body to build the `AccessPolicy { allowed_statuses: &[Active, Honorary], require_admin: false, enforce_admin_totp: false, on_reject: Json401 }`, call `authenticate`, on Ok inject `CurrentUser` + `SessionInfo` and forward, on Err return the appropriate error via `AppError`. Preserve the existing `Result<Response, AppError>` signature.
- [x] 2.2 Confirm no `SqliteMemberRepository::new(...)` remains in this function.
- [x] 2.3 Run the existing scenario tests for `require_auth` (anonymous → 401, Pending → 403, Active → forwarded).

## 3. Migrate `require_auth_redirect`, `require_restorable`, `require_admin_redirect`

- [x] 3.1 Rewrite `require_auth_redirect` to use `AccessPolicy { allowed_statuses: &[Active, Honorary], require_admin: false, enforce_admin_totp: false, on_reject: RedirectToRestoreOrLogin }`. Preserve the existing `Response` signature.
- [x] 3.2 Rewrite `require_restorable` to use `AccessPolicy { allowed_statuses: &[Active, Honorary, Expired], require_admin: false, enforce_admin_totp: false, on_reject: RedirectToLogin }`.
- [x] 3.3 Rewrite `require_admin_redirect` to use `AccessPolicy { allowed_statuses: &[Active, Honorary], require_admin: true, enforce_admin_totp: true, on_reject: RedirectToDashboardOrLogin }`.
- [x] 3.4 Confirm no `SqliteMemberRepository::new(...)` remains in any of these three functions.
- [x] 3.5 Run the existing scenario tests in the `auth-middleware-tiers` spec: Expired hitting `require_auth_redirect` → `/portal/restore`; Expired hitting `require_restorable` → forwarded; non-admin hitting `require_admin_redirect` → `/portal/dashboard`; admin without TOTP when setting on → `/portal/profile/security?reason=admin_totp_required`; setting-lookup error → not enforced.

## 4. Migrate `optional_auth`

- [x] 4.1 Rewrite `optional_auth` to call the shared `authenticate(...)` with a permissive policy (all statuses allowed, no admin), and on Ok inject `CurrentUser`. On any rejection silently forward without injecting.
- [x] 4.2 Confirm `SqliteMemberRepository::new(...)` no longer appears in `src/api/middleware/auth.rs`.

## 5. Verify and clean up

- [x] 5.1 Run `cargo build` and `cargo test --features test-utils`. Every existing test should pass; if any fail, that's a behavior drift and SHALL be fixed before merge (not the test).
- [x] 5.2 Add a unit test `auth::tests::access_policy_matrix` that, for each of the four wrappers, exercises the (anonymous, Pending, Suspended, Expired, Active-non-admin, Active-admin, Active-admin-no-TOTP-with-setting-on) matrix and asserts the expected reject behavior or forward. (Implemented as an integration test in `tests/auth_middleware_test.rs::access_policy_matrix` because the wrappers need a full `AppState` to exercise — the spirit of the matrix-coverage requirement is met.)
- [x] 5.3 Confirm `src/api/mod.rs`, `src/web/portal/mod.rs`, and `src/web/mod.rs` were not touched (signatures stable).
- [x] 5.4 Eyeball the final `auth.rs` line count — expected target ~140 lines (down from 273). Actual ~277 lines: the wrapper bodies collapsed from ~30 lines to 3 lines each, but the type/policy declarations + the shared `authenticate`/`render_reject`/`gate` helpers add roughly the same total back. The structural win (single core + table of policies) is preserved.

## 6. Spec sync

- [x] 6.1 Confirm the change's delta spec (`openspec/changes/consolidate-auth-middleware/specs/auth-middleware-tiers/spec.md`) matches the implemented behavior.
- [ ] 6.2 At archive time (`opsx:archive`), the new requirements (shared core, shared repo, stable signatures) merge into `openspec/specs/auth-middleware-tiers/spec.md`.
