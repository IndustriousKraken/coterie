## Context

`src/api/middleware/auth.rs` (273 lines) defines five middleware variants. The four gating variants (`require_auth`, `require_auth_redirect`, `require_restorable`, `require_admin_redirect`) all do the same five steps:

```
1. jar.get("session")              → cookie
2. auth_service.validate_session() → SessionRow
3. SqliteMemberRepository::new + find_by_id → Member
4. match member.status against allowed set
5. request.extensions_mut().insert(CurrentUser + SessionInfo)
```

The variation between them, summarized:

| Middleware                | Allowed                    | On reject                                 | Extra check                          |
|---------------------------|----------------------------|-------------------------------------------|--------------------------------------|
| `require_auth`            | Active, Honorary           | Pending → 403; rest → 401                 | —                                    |
| `require_auth_redirect`   | Active, Honorary           | Expired → `/portal/restore`; rest → login | —                                    |
| `require_restorable`      | Active, Honorary, Expired  | Anything else → login                     | —                                    |
| `require_admin_redirect`  | Active, Honorary           | Anything else → login                     | `is_admin` + optional TOTP enrollment |

`optional_auth` has the same shape but tolerates failure at every step.

Two friction points today:

- **Drift surface**. A change to step 1, 2, 3, or 5 has to land in 4–5 places. The TOTP-gate-only-on-admin is the kind of thing that *could* drift if a sixth variant were added — there's no structural enforcement that step ordering and contents stay in sync.
- **Repository duplication**. Each middleware does `SqliteMemberRepository::new(state.service_context.db_pool.clone())` inline rather than using `state.service_context.member_repo`. Two parallel paths to the same data.

The existing `auth-middleware-tiers` spec is well-written and treats each named middleware as an externally-visible contract. That's what we want to preserve — the named functions stay, their reject behavior stays exact, but their bodies become thin policy declarations over a shared core.

## Goals / Non-Goals

**Goals:**
- One canonical implementation of the cookie-validate-load-inject sequence.
- Per-variant differences expressed as *data* (allowed statuses, on-reject behavior, admin/TOTP flags), not as duplicated imperative code.
- Zero wire-shape change: identical redirect targets, status codes, header semantics, and extension types.
- Public function names (`require_auth`, `require_auth_redirect`, `require_restorable`, `require_admin_redirect`, `optional_auth`) remain unchanged so router files don't churn.
- The shared `state.service_context.member_repo` Arc is used; per-call `SqliteMemberRepository::new(...)` constructions go away.

**Non-Goals:**
- Adding new middleware variants (e.g., a member-or-pending variant). Every existing variant maps 1:1 to a variant after the change.
- Changing the auth-service or repository surface. The session-validation API and `MemberRepository::find_by_id` stay.
- Touching `setup::require_setup` (it doesn't fit the auth-middleware pattern; it's about first-boot routing).
- Async refactor or session rotation — those are separate concerns.
- Lifting the TOTP-enforcement check to its own middleware (it's logically part of admin gating; splitting it would be premature).

## Decisions

### D1. `AccessPolicy` is a small struct, not a trait

```rust
struct AccessPolicy {
    allowed_statuses: &'static [MemberStatus],
    require_admin: bool,
    enforce_admin_totp: bool,
    on_reject: RejectBehavior,
}
```

Considered: a trait with associated functions per variant. Rejected — overkill for four call sites, harder to read at the wrapper site, and prevents the table-of-policies legibility we want.

### D2. `RejectBehavior` is an enum of four variants

```rust
enum RejectBehavior {
    Json401,                       // require_auth
    RedirectToLogin,               // require_restorable, anything not Expired
    RedirectToRestoreOrLogin,      // require_auth_redirect
    RedirectToDashboardOrLogin,    // require_admin_redirect (non-admin sees dashboard)
}
```

The enum captures the four observable reject behaviors today. Each carries the complete decision tree (e.g., `RedirectToRestoreOrLogin` knows to send Expired to `/portal/restore` and everything else to login). The shared core `match`es on this once at the end; no per-wrapper if/else.

`require_auth` has a subtler behavior: Pending → 403, anonymous/expired/suspended → 401. That stays inside the `Json401` branch — the core returns the right code based on whether the member loaded successfully and what their status is. Existing scenario "Pending member receives 403" is preserved.

### D3. The shared core returns a `Result<Authenticated, RejectReason>` shape

```rust
struct Authenticated {
    member: Member,
    session_id: String,
}

enum RejectReason {
    NoCookie,
    InvalidSession,
    MemberNotFound,
    StatusBlocked(MemberStatus),
    NotAdmin,
    AdminTotpMissing,
}
```

The core does steps 1–4. `RejectReason` carries enough information that the on-reject branch can render the right response without re-deriving anything. The existing `redirect_to_login` helper stays.

### D4. The admin-only TOTP check stays inline in the core, gated by `policy.enforce_admin_totp`

When `require_admin` is true and the policy turns on TOTP enforcement, the core reads `auth.require_totp_for_admins` and checks the member's TOTP enrollment. This matches today's behavior exactly, including the soft-fail-to-not-enforced semantics if the setting lookup errors (preserves the "Setting lookup failure defaults to not enforced" scenario).

### D5. `optional_auth` calls the same core but ignores rejections

```rust
pub async fn optional_auth(...) -> Response {
    if let Ok(auth) = authenticate(/* allow-all policy */).await {
        request.extensions_mut().insert(CurrentUser { member: auth.member });
    }
    next.run(request).await
}
```

Considered: keeping `optional_auth` open-coded since it doesn't reject. Rejected — it benefits from the same dedup; and if session validation gets a fix in the future, we want it to apply here too.

### D6. Use `state.service_context.member_repo` directly

Existing code constructs `SqliteMemberRepository::new(state.service_context.db_pool.clone())` per call. The `ServiceContext` already holds `Arc<dyn MemberRepository>`. Switch every call to use the shared Arc. This is technically observable in tests that swap in a fake — today's middleware always hits SQLite no matter what test wiring exists.

### D7. Public function names and signatures are preserved

Every router still imports and layers `require_auth`, `require_auth_redirect`, etc. by the same name and signature. This is a property worth defending: if signatures change, every router file in the tree changes, and so does every test using `from_fn_with_state`. The wrappers exist explicitly to keep the call sites stable.

### D8. Wrappers are tiny but present

The wrappers don't disappear — they're the named symbols routers reference. Each wrapper is ~10 lines: build the `AccessPolicy`, call `authenticate(...)`, branch on the result. That's the legibility win: a reader looking at `require_admin_redirect` sees the policy at a glance instead of scrolling through 70 lines of duplicate cookie/session/member plumbing.

### D9. Module organization stays flat

Everything stays in `src/api/middleware/auth.rs`. The `AccessPolicy`, `RejectBehavior`, `Authenticated`, and `RejectReason` types are private to the module. No new files, no submodule split. The file shrinks from ~273 to ~140 lines, well within "single file" comfort.

## Risks / Trade-offs

- **Risk**: subtle behavior drift during the move (e.g., an off-by-one in which redirect a particular `RejectReason` resolves to). → **Mitigation**: every existing `auth-middleware-tiers` spec scenario has a clear input/output; map them to unit tests against the wrapper functions and run them before and after.
- **Risk**: the `&'static [MemberStatus]` slices in `AccessPolicy` add minor lifetime ceremony. → **Mitigation**: each wrapper's policy is a `const`. The compiler enforces correctness; no runtime cost.
- **Trade-off**: indirection on read. A reader following `require_admin_redirect` jumps to `authenticate(...)` to see the full body. The trade-off is justified by the shared core being authoritative for session/member loading — the same property that makes the change valuable.
- **Trade-off**: a future variant that needs *more* than the four current axes (e.g., "admin-or-self" for member-detail pages) can't be expressed in the policy. That's acceptable: at that point we add an axis or split out a bespoke middleware. The structure makes either choice cheap.

## Migration Plan

Pure-internal refactor; one PR.

1. Land the `authenticate(...)` core, `AccessPolicy`, `RejectBehavior`, `RejectReason`, `Authenticated` types alongside the existing middlewares.
2. Rewrite each wrapper to delegate, one at a time. Compile and test after each.
3. Delete the old function bodies (the wrappers replace them in place).
4. Sweep the file for `SqliteMemberRepository::new(...)` and remove.
5. Run the full test suite (`cargo test --features test-utils`).
6. Deploy normally — no migrations, no flags. `git revert` is the rollback.
