## 1. Service skeleton

- [ ] 1.1 Create `src/service/member_service.rs` with `MemberService` struct, dependencies (`MemberRepository`, `AuthService`, `AuditService`, `IntegrationManager`, `EmailSender`, `MembershipTypeService`, `SettingsService`, `EmailTokenService`), and `new(...)` constructor. Mirror `PaymentService`'s shape.
- [ ] 1.2 Register `pub mod member_service;` in `src/service/mod.rs`.
- [ ] 1.3 Add `pub member_service: Arc<MemberService>` field to `ServiceContext` and construct it inside `ServiceContext::new`. Wire all required deps (note: `EmailTokenService` and `SettingsService` are already constructed earlier in `ServiceContext::new`).
- [ ] 1.4 Verify `cargo build` still passes with the empty-shell service plumbed but unused.

## 2. Migrate `activate`

- [ ] 2.1 Add `MemberService::activate(actor_id: Uuid, member_id: Uuid) -> Result<Member>` that does: repo update to Active → invalidate sessions (log+swallow on failure) → audit log → `MemberActivated` integration dispatch → welcome email (log+swallow on failure) → return new member.
- [ ] 2.2 Move `send_welcome_email` from `members.rs` into `member_service.rs` as a private method on `MemberService`.
- [ ] 2.3 Rewrite `admin_activate_member` in `src/web/portal/admin/members.rs`: parse uuid → call `state.service_context.member_service.activate(current_user.member.id, id)` → render `partials::member_row_flash` on Ok / `partials::member_row_error` on Err.
- [ ] 2.4 Add unit test `member_service::tests::activate_emits_full_chain` asserting all five side-effects fire (repo touched, sessions invalidated, audit row inserted, integration event dispatched, email sent) using existing fakes / in-memory repos.
- [ ] 2.5 Verify existing handler-level tests for activate still pass.

## 3. Migrate `suspend`, `update`, `extend_dues`, `set_dues`, `expire_now`

- [ ] 3.1 Add `MemberService::suspend(actor_id, member_id) -> Result<Member>` mirroring the snapshot → repo update → invalidate sessions → audit → `MemberUpdated { old, new }` chain.
- [ ] 3.2 Add `MemberService::update(actor_id, member_id, request: UpdateMemberRequest) -> Result<Member>` with the same shape.
- [ ] 3.3 Add `MemberService::extend_dues(actor_id, member_id, months: i32) -> Result<Member>`. Validate `1..=120` inside the service and return `AppError::BadRequest` on out-of-range.
- [ ] 3.4 Add `MemberService::set_dues(actor_id, member_id, naive_date) -> Result<Member>`.
- [ ] 3.5 Add `MemberService::expire_now(actor_id, member_id) -> Result<Member>` doing `expire_dues_now` → invalidate sessions → audit → `MemberUpdated`.
- [ ] 3.6 Move `dispatch_member_updated` helper from `members.rs` into `member_service.rs` as a private method (and verify there are no remaining call sites in handlers).
- [ ] 3.7 Rewrite `admin_suspend_member`, `admin_update_member`, `admin_extend_dues`, `admin_set_dues`, `admin_expire_now` to delegate to the service.
- [ ] 3.8 Add a unit test per method asserting the expected side-effect chain.

## 4. Migrate `update_discord_id`, `resend_verification`, `create`

- [ ] 4.1 Add `MemberService::update_discord_id(actor_id, member_id, discord_id: Option<String>) -> Result<Member>` doing snowflake validation (currently in handler) → repo update_discord_id → audit → `MemberUpdated`.
- [ ] 4.2 Add `MemberService::resend_verification(actor_id, member_id) -> Result<()>` doing token issue → email send → audit. Email failures are logged-but-non-fatal.
- [ ] 4.3 Add `MemberService::create(actor_id, request: CreateMemberRequest) -> Result<Member>` doing repo create → welcome email (log+swallow) → audit. No `MemberActivated` event (the create path produces Pending; activation event fires on later activate).
- [ ] 4.4 Rewrite `admin_update_discord_id`, `admin_resend_verification`, `admin_create_member` to delegate.
- [ ] 4.5 Verify the welcome-email, verification-email, and discord_id-validation helpers in `members.rs` are no longer referenced and remove them.
- [ ] 4.6 Add unit tests for the three new methods.

## 5. Confirm scope boundaries and clean up

- [ ] 5.1 Verify `admin_refund_payment` is unchanged (out of scope per design.md).
- [ ] 5.2 Confirm no handler in `src/web/portal/admin/members.rs` calls `state.service_context.audit_service.log`, `state.service_context.integration_manager.handle_event`, or `state.service_context.auth_service.invalidate_all_sessions` directly for a member-mutation flow. (The refund handler may still call audit_service — that's fine.)
- [ ] 5.3 Run `cargo test --features test-utils` and confirm the full suite passes.
- [ ] 5.4 Eyeball the final `members.rs` line count — expected target ~800–900 lines (down from 1586) once the inline orchestration is extracted.

## 6. Spec sync

- [ ] 6.1 Confirm the change's delta specs (under `openspec/changes/lift-member-admin-orchestration/specs/`) match the implemented behavior.
- [ ] 6.2 At archive time (`opsx:archive`), the new `member-admin-service` capability spec lands under `openspec/specs/member-admin-service/spec.md`, and the deltas to `admin-members`, `audit-logging`, `integration-events` are merged into their canonical specs.
