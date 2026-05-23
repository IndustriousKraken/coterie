## 1. Create submodule files

- [ ] 1.1 Create `list.rs`, `detail.rs`, `create.rs`, `status.rs`, `dues.rs`, `payments.rs`, `discord.rs`, `verification.rs` in `src/web/portal/admin/members/`.

## 2. Move handlers per the inventory

- [ ] 2.1 `list.rs`: `admin_members_page` (current line 88).
- [ ] 2.2 `status.rs`: `admin_activate_member` (227), `admin_suspend_member` (247), `admin_expire_now` (826).
- [ ] 2.3 `detail.rs`: `admin_member_detail_page` (307), `admin_update_member` (414).
- [ ] 2.4 `payments.rs`: `admin_record_payment_page` (484), `admin_record_payment_submit` (564), `parse_dollars_to_cents` (680, private), `rerender_with_error` (701, private).
- [ ] 2.5 `dues.rs`: `admin_extend_dues` (754), `admin_set_dues` (790), `admin_member_payments` (845).
- [ ] 2.6 `create.rs`: `admin_new_member_page` (873), `admin_create_member` (911).
- [ ] 2.7 `discord.rs`: `admin_update_discord_id` (1003), `discord_id_result` (1035, private).
- [ ] 2.8 `verification.rs`: `admin_resend_verification` (1048), `resend_result` (1078, private).

## 3. Reconcile imports

- [ ] 3.1 For each new submodule, add the `use` statements its handlers need. Start by copying the current `mod.rs` `use` block and prune per-submodule.
- [ ] 3.2 `cargo build` will flag any missing imports; resolve.

## 4. Update mod.rs

- [ ] 4.1 Strip `mod.rs` down to:
   - `use` block for whatever `mod.rs` itself still needs (Axum types for the router)
   - `mod list; mod detail; mod create; mod status; mod dues; mod payments; mod discord; mod verification;`
   - The `routes()` function (or equivalent) wiring paths to `list::admin_members_page`, etc.
- [ ] 4.2 Update handler references in `routes()` to use their new module paths.

## 5. Visibility check

- [ ] 5.1 For each handler, try `pub(super) async fn` first. If `cargo build` errors with a visibility issue, escalate to `pub async fn`. Use the narrowest visibility that compiles.

## 6. Validation

- [ ] 6.1 `cargo build --features test-utils` — clean compile.
- [ ] 6.2 `cargo test --features test-utils` — all tests pass.
- [ ] 6.3 `cargo clippy --features test-utils -- --deny warnings` — clean.
- [ ] 6.4 `cargo fmt --check` — clean.
- [ ] 6.5 `wc -l src/web/portal/admin/members/*.rs` — confirm no file exceeds 300 lines.
