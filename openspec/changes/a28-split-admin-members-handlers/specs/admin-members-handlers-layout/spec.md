## ADDED Requirements

### Requirement: Admin member handlers are organized into focused submodules

`src/web/portal/admin/members/mod.rs` SHALL be reduced to route registration and module declarations. The handler functions SHALL be moved into focused submodules under `src/web/portal/admin/members/`:

- `mod.rs` — router + module declarations
- `list.rs` — member browse/listing handlers
- `detail.rs` — member detail view + update
- `create.rs` — new-member form + submit
- `status.rs` — activate, suspend, expire-now
- `dues.rs` — extend-dues, set-dues, member-payments view
- `payments.rs` — record-payment page/submit + their local helpers (`parse_dollars_to_cents`, `rerender_with_error`)
- `discord.rs` — discord-id update + result fragment helper
- `verification.rs` — resend-verification + result fragment helper

Each handler's visibility SHALL be the narrowest that satisfies the router's needs (`pub(super)` preferred where it works; `pub` otherwise).

#### Scenario: No submodule exceeds 300 lines

- **WHEN** the post-split files in `src/web/portal/admin/members/` are line-counted
- **THEN** every file SHALL be ≤300 lines

#### Scenario: All routes still resolve to the same handlers

- **WHEN** integration tests that hit admin-member routes are run
- **THEN** they SHALL pass without modification; URL → handler resolution behavior is unchanged

#### Scenario: Local helpers stay with their callers

- **WHEN** `parse_dollars_to_cents` and `rerender_with_error` are located after the split
- **THEN** they SHALL live in `payments.rs` (the only submodule that uses them) as private functions, NOT promoted to a shared `helpers.rs` or `mod.rs`
