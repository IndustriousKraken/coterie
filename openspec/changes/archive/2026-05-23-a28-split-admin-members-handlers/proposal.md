## Why

`src/web/portal/admin/members/mod.rs` is 1090 lines holding ~17 admin-member handler functions covering member browse, detail/edit, create, status transitions, dues admin, payment recording, Discord linking, and email-verification resend. Because it's already a module directory (`admin/members/`), splitting is trivial — `mod.rs` just declares submodules and the handlers move to them.

Splitting reduces review surface, makes each handler concern self-contained, and follows the pattern already used for `admin/events/`, `admin/announcements/`, etc.

## What Changes

- Split `src/web/portal/admin/members/mod.rs` into submodules. `mod.rs` shrinks to just route registration + module declarations.
- Proposed submodule layout (function inventory from grep at spec time):
  - `mod.rs` — router, `mod` declarations
  - `list.rs` — `admin_members_page` (line 88)
  - `detail.rs` — `admin_member_detail_page` (307), `admin_update_member` (414)
  - `create.rs` — `admin_new_member_page` (873), `admin_create_member` (911)
  - `status.rs` — `admin_activate_member` (227), `admin_suspend_member` (247), `admin_expire_now` (826)
  - `dues.rs` — `admin_extend_dues` (754), `admin_set_dues` (790), `admin_member_payments` (845)
  - `payments.rs` — `admin_record_payment_page` (484), `admin_record_payment_submit` (564), `parse_dollars_to_cents` (680), `rerender_with_error` (701)
  - `discord.rs` — `admin_update_discord_id` (1003), `discord_id_result` (1035)
  - `verification.rs` — `admin_resend_verification` (1048), `resend_result` (1078)

## Capabilities

### New Capabilities
- `admin-members-handlers-layout`: admin member handlers are organized into focused submodules under `admin/members/`, mirroring how other admin areas are structured.

### Modified Capabilities
None.

## Impact

- **Code**: net-neutral.
- **Wire shape**: zero change — routes resolve to the same handlers, just imported from different files.
- **Tests**: no test changes needed; integration tests hit routes, not handler function paths.
- **Risk**: low. Mechanical split. Risks: `pub` vs `pub(super)` visibility on handlers (Axum needs `async fn` handlers to be public-enough for the router to reference them).
- **Dependency**: none.
