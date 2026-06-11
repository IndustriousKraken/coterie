## 1. Move `DonationCampaignRepository` (smallest trait first)

- [x] 1.1 In `src/repository/mod.rs`, locate `pub trait DonationCampaignRepository: Send + Sync { ... }` (around line 317). Cut the entire trait block.
- [x] 1.2 Paste it into `src/repository/donation_repository.rs` just above the existing `impl ... for SqliteDonationCampaignRepository` block.
- [x] 1.3 In `donation_repository.rs`, remove `DonationCampaignRepository` from any `use crate::repository::{...}` import line that's now self-referential.
- [x] 1.4 In `mod.rs`'s re-export block, change `pub use donation_repository::SqliteDonationCampaignRepository;` to `pub use donation_repository::{DonationCampaignRepository, SqliteDonationCampaignRepository};`.
- [x] 1.5 `cargo build` — green.

## 2. Move `SavedCardRepository`

- [x] 2.1 Cut `pub trait SavedCardRepository: Send + Sync { ... }` from `mod.rs` and paste into `saved_card_repository.rs` above the existing impl block.
- [x] 2.2 Adjust `saved_card_repository.rs`'s imports — drop the self-referential `SavedCardRepository`.
- [x] 2.3 Update `mod.rs`'s `pub use` to export both the trait and the impl from `saved_card_repository`.
- [x] 2.4 `cargo build` — green.

## 3. Move `ScheduledPaymentRepository`

- [x] 3.1 Cut `pub trait ScheduledPaymentRepository: Send + Sync { ... }` from `mod.rs` and paste into `scheduled_payment_repository.rs`.
- [x] 3.2 Adjust imports in `scheduled_payment_repository.rs`.
- [x] 3.3 Update `mod.rs`'s `pub use`.
- [x] 3.4 `cargo build` — green.

## 4. Move `AnnouncementRepository`

- [x] 4.1 Cut `pub trait AnnouncementRepository: Send + Sync { ... }` from `mod.rs` and paste into `announcement_repository.rs`.
- [x] 4.2 Adjust imports in `announcement_repository.rs`.
- [x] 4.3 Update `mod.rs`'s `pub use`.
- [x] 4.4 `cargo build` — green.

## 5. Move `EventRepository`

- [x] 5.1 Cut `pub trait EventRepository: Send + Sync { ... }` from `mod.rs` and paste into `event_repository.rs`.
- [x] 5.2 Adjust imports in `event_repository.rs`.
- [x] 5.3 Update `mod.rs`'s `pub use`.
- [x] 5.4 `cargo build` — green.

## 6. Move `PaymentRepository` and `MonthlyRevenue`

- [x] 6.1 Cut `pub trait PaymentRepository: Send + Sync { ... }` from `mod.rs` and paste into `payment_repository.rs` above the existing impl block.
- [x] 6.2 Cut `pub struct MonthlyRevenue { ... }` (with its full doc comment) from `mod.rs` and paste into `payment_repository.rs` near the trait.
- [x] 6.3 In `payment_repository.rs`, remove `PaymentRepository` and `MonthlyRevenue` from the `use crate::repository::{...}` import — they're now self-referential.
- [x] 6.4 Update `mod.rs`'s `pub use payment_repository::SqlitePaymentRepository;` to `pub use payment_repository::{PaymentRepository, SqlitePaymentRepository, MonthlyRevenue};`.
- [x] 6.5 `cargo build` — green.

## 7. Move `MemberRepository` and its auxiliary types

- [x] 7.1 Cut `pub trait MemberRepository: Send + Sync { ... }` (with all its doc comments) from `mod.rs` and paste into `member_repository.rs` above the existing impl block.
- [x] 7.2 Cut `pub struct MemberQuery { ... }` (with its multi-line doc comment) from `mod.rs` and paste into `member_repository.rs` near the trait.
- [x] 7.3 Cut `pub enum MemberSortField { ... }` and `pub enum SortOrder { ... }` from `mod.rs` and paste into `member_repository.rs`.
- [x] 7.4 In `member_repository.rs`, simplify the `use crate::{...}` block: drop `repository::{MemberQuery, MemberRepository, MemberSortField, SortOrder}` — all four are now in the same file.
- [x] 7.5 Update `mod.rs`'s `pub use member_repository::SqliteMemberRepository;` to `pub use member_repository::{MemberRepository, SqliteMemberRepository, MemberQuery, MemberSortField, SortOrder};`.
- [x] 7.6 `cargo build` — green.

## 8. Verify the final shape

- [x] 8.1 `wc -l src/repository/mod.rs` — expected ~40 lines.
- [x] 8.2 Grep `mod.rs` for `pub trait`, `pub struct`, `pub enum`, `impl` — all should return zero matches.
- [x] 8.3 Confirm `mod.rs` contains only `pub mod` and `pub use` lines (a few comment lines or blank lines are fine).
- [x] 8.4 Run `cargo test --features test-utils` — full suite passes without modification.
- [x] 8.5 Confirm `crate::repository::MemberRepository`, `crate::repository::MemberQuery`, `crate::repository::MonthlyRevenue`, and the rest still resolve in their existing call sites (`src/auth/mod.rs`, `src/web/portal/admin/members.rs`, `src/service/mod.rs`).

## 9. Spec sync

- [x] 9.1 Confirm the change's delta spec (`openspec/changes/a04-move-repo-traits-to-per-file/specs/repository-contracts/spec.md`) matches the implemented behavior.
