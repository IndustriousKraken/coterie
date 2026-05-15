## Why

`src/repository/mod.rs` is 321 lines of trait declarations plus a few auxiliary types. The repository module follows two patterns inconsistently:

**Pattern A — trait in `mod.rs`, impl in the per-file module** (7 traits):
- `MemberRepository`, `EventRepository`, `AnnouncementRepository`, `PaymentRepository`, `SavedCardRepository`, `ScheduledPaymentRepository`, `DonationCampaignRepository`

**Pattern B — trait + impl both in the per-file module** (5 traits):
- `EventSeriesRepository`, `EventTypeRepository`, `AnnouncementTypeRepository`, `MembershipTypeRepository`, `ProcessedEventsRepository`

The current `repository-contracts` spec hardcodes Pattern A as a requirement: *"Each repository SHALL have a trait declared in `src/repository/mod.rs` and an implementation in the matching `<entity>_repository.rs`."* The actual code violates this for 5 out of 12 repositories.

Pattern B is the better default because:

- A reader looking at `EventTypeRepository` finds the trait, the impl, the row struct, and any helpers in one file. Following the trait in Pattern A means jumping back to `mod.rs` and then forward to the impl file.
- Adding a new repo method in Pattern A touches two files (the trait in `mod.rs`, the impl in the per-file module). In Pattern B it touches one. The split makes diffs noisier without buying anything.
- `mod.rs` would shrink from a 321-line trait dump to a one-liner-per-repo index — easier to scan, less prone to merge conflicts, and the file's role becomes "module index" rather than "trait dump + module index."

The fix is to move every Pattern-A trait into its per-file module (alongside its impl), move the auxiliary types (`MemberQuery`, `MemberSortField`, `SortOrder`, `MonthlyRevenue`) next to the trait that uses them, and reduce `mod.rs` to `pub mod` declarations plus `pub use` re-exports. Public import paths (`crate::repository::MemberRepository`, `crate::repository::MonthlyRevenue`, etc.) are preserved by the re-exports — no callers move.

## What Changes

- **Move 7 traits from `src/repository/mod.rs` to their matching per-file modules**:
  - `MemberRepository` → `member_repository.rs`
  - `EventRepository` → `event_repository.rs`
  - `AnnouncementRepository` → `announcement_repository.rs`
  - `PaymentRepository` → `payment_repository.rs`
  - `SavedCardRepository` → `saved_card_repository.rs`
  - `ScheduledPaymentRepository` → `scheduled_payment_repository.rs`
  - `DonationCampaignRepository` → `donation_repository.rs`
- **Move 4 auxiliary types alongside the traits that use them**:
  - `MemberQuery`, `MemberSortField`, `SortOrder` → `member_repository.rs` (used only by `MemberRepository::search`).
  - `MonthlyRevenue` → `payment_repository.rs` (used only by `PaymentRepository::revenue_by_month`).
- **Re-export everything from `mod.rs`** so `crate::repository::MemberRepository`, `crate::repository::MemberQuery`, `crate::repository::MonthlyRevenue`, etc. continue to resolve. Pattern: one `pub use` per module covering both the trait, the impl, and any auxiliary types in that module.
- **Sweep imports inside the per-file modules** that previously read `use crate::repository::{MemberRepository, MemberQuery, ...}`. Those imports become self-referential after the trait moves; remove or simplify.
- **Spec delta**: `repository-contracts`'s "trait declared in mod.rs" requirement updates to "trait declared in the per-file module." The "single source of truth" intent stays the same; the location moves.

## Capabilities

### New Capabilities

(None — internal restructuring of an existing capability. No new spec file.)

### Modified Capabilities
- `repository-contracts`: the file-location requirement updates. "Each repository's trait is declared in its per-file module alongside the impl" replaces "declared in `mod.rs`." Other contracts (strongly-typed query inputs, idempotency documentation, in-memory SQLite tests) are unchanged.

## Impact

- **Code**:
  - `src/repository/mod.rs` shrinks from 321 lines to ~40 lines (a `pub mod` block plus a `pub use` block).
  - 7 per-file modules grow by ~10–80 lines each (the trait moves in; for `MemberRepository` and `PaymentRepository` the auxiliary types come too).
  - Internal imports inside the per-file modules simplify. ~5 lines of `use crate::repository::{...}` self-references go away.
  - Net line change: roughly zero (lines move, total stays the same), but `mod.rs` becomes legible at a glance.
- **Wire shape**: zero change. Pure file-organization refactor. Same SQL, same trait methods, same DB rows.
- **Public import paths**: preserved by re-exports. No call site outside `src/repository/` needs to update.
- **Tests**: existing tests pass without modification — they import via `crate::repository::*` or specific names, all of which keep resolving. No new tests required (this is a structural change, not a behavioral one).
- **Risk**: trivial. Compiler enforces correctness at every step. Mistakes manifest as build errors, not runtime regressions.
- **Trade-off accepted**: per-file modules grow longer. `payment_repository.rs` is already 510 lines; absorbing the trait + `MonthlyRevenue` adds ~80 lines. Still well within "single file" comfort.
