## Why

`src/api/middleware/auth.rs` defines five middleware variants — `require_auth`, `require_auth_redirect`, `require_restorable`, `require_admin_redirect`, `optional_auth` — that each repeat the same five mechanical steps: read session cookie → validate session → load member → check status → inject `CurrentUser` and `SessionInfo`. The only meaningful axes of variation are (a) the allowed status set, (b) whether to 401 or redirect on reject, and (c) for the admin variant, an additional `is_admin` + optional TOTP enforcement step.

Today this lives as ~250 lines of mechanical copy-paste across five functions. Two concrete consequences:

1. A change to session validation (e.g., adding a "rotate session id on use" step) has to land in five places. The drift surface is real: the `require_admin_redirect`-only TOTP gate is exactly the kind of thing that could plausibly fail to propagate to a sixth variant if one were added.
2. Every variant constructs a fresh `SqliteMemberRepository::new(state.service_context.db_pool.clone())` instead of using the shared `state.service_context.member_repo` Arc. Two parallel paths to the same data, neither obviously canonical.

A single shared core with a small `AccessPolicy` parameter (allowed statuses, on-reject behaviour, admin gate flag) collapses the duplication while keeping each named middleware as a thin wrapper over the core. Wire-visible behaviour is unchanged: same redirects, same 401/403 responses, same TOTP enforcement, same `CurrentUser` injection.

## What Changes

- **Add a private `authenticate(...)` helper** in `src/api/middleware/auth.rs` that performs the shared steps once: read cookie → validate session → load member via `state.service_context.member_repo` (not a freshly-constructed repo) → return `Result<(Member, SessionInfo), AuthFailure>`.
- **Add a private `AccessPolicy` struct** that captures the per-variant axes:
  - `allowed_statuses: &'static [MemberStatus]`
  - `require_admin: bool`
  - `enforce_admin_totp: bool` (only meaningful when `require_admin`)
  - `on_reject: RejectBehavior` — enum: `Json401`, `RedirectToLogin`, `RedirectToRestoreOrLogin`, `RedirectToDashboardOrLogin`
- **Rewrite the four gating middlewares** (`require_auth`, `require_auth_redirect`, `require_restorable`, `require_admin_redirect`) as ~10-line wrappers that build their `AccessPolicy` and delegate to the shared core.
- **Rewrite `optional_auth`** to use the same `authenticate(...)` helper, ignoring failures (it intentionally tolerates anonymous requests).
- **Remove the per-middleware `SqliteMemberRepository::new(...)` calls** — use `state.service_context.member_repo` everywhere.
- Behavior contract: every existing scenario in the `auth-middleware-tiers` spec continues to pass without modification. Specifically: redirects to `/portal/restore` for Expired hitting `require_auth_redirect`, redirects to `/portal/dashboard` for non-admin hitting `require_admin_redirect`, redirects to `/portal/profile/security?reason=admin_totp_required` for un-enrolled admin when the setting is on, 401/403 (not redirects) for `require_auth`.

## Capabilities

### New Capabilities

(None — this is an internal refactor of an existing capability. No new spec file.)

### Modified Capabilities
- `auth-middleware-tiers`: adds an internal-structure requirement (the four gating variants share a single core implementation; per-variant policies are declared as data, not duplicated as imperative code). Externally-observable behavior — the existing scenarios about redirects, 401s, admin gating, and TOTP enforcement — is unchanged. The delta documents the new shared-core constraint so a future contributor can't reintroduce divergent code paths.

## Impact

- **Code**: `src/api/middleware/auth.rs` shrinks from ~273 lines to roughly 130–150 lines. No other files change beyond the import paths used by the routers (`src/api/mod.rs`, `src/web/portal/mod.rs`, `src/web/mod.rs`) — public function names and signatures stay the same so callers don't move.
- **Wire shape**: zero change. Same redirect URLs, same status codes, same `CurrentUser`/`SessionInfo` extensions, same TOTP enforcement.
- **Tests**: existing handler-level and integration tests that rely on auth behavior continue to pass. Add unit tests for the `AccessPolicy` matrix (one per variant) confirming the four gating middlewares produce the expected reject behavior for anonymous, Pending, Suspended, Expired, Active-non-admin, and Active-admin members.
- **Risk**: low — the change is mechanical, the contract is already well-specified, and no caller signatures change. Mitigation: the new core is exercised by every existing auth-related test the moment it lands.
- **Bonus cleanup**: the parallel `SqliteMemberRepository::new` path goes away. Future repository-trait swaps (e.g., for testing) only have to be wired through `service_context.member_repo`.
