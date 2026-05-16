## Context

The repository module has 12 repositories. Today their traits live in two different places:

| Repository                    | Trait location           | Impl location                                |
|-------------------------------|--------------------------|----------------------------------------------|
| `MemberRepository`            | `mod.rs:67`              | `member_repository.rs`                       |
| `EventRepository`             | `mod.rs:146`             | `event_repository.rs`                        |
| `AnnouncementRepository`      | `mod.rs:190`             | `announcement_repository.rs`                 |
| `PaymentRepository`           | `mod.rs:202`             | `payment_repository.rs`                      |
| `SavedCardRepository`         | `mod.rs:288`             | `saved_card_repository.rs`                   |
| `ScheduledPaymentRepository`  | `mod.rs:298`             | `scheduled_payment_repository.rs`            |
| `DonationCampaignRepository`  | `mod.rs:317`             | `donation_repository.rs`                     |
| `EventSeriesRepository`       | `event_series_repository.rs` | `event_series_repository.rs`             |
| `EventTypeRepository`         | `event_type_repository.rs`   | `event_type_repository.rs`               |
| `AnnouncementTypeRepository`  | `announcement_type_repository.rs` | `announcement_type_repository.rs`   |
| `MembershipTypeRepository`    | `membership_type_repository.rs`   | `membership_type_repository.rs`     |
| `ProcessedEventsRepository`   | `processed_events_repository.rs`  | `processed_events_repository.rs`    |

Plus four auxiliary types in `mod.rs` that ride alongside specific traits:

- `MemberQuery`, `MemberSortField`, `SortOrder` — input shape and helpers for `MemberRepository::search`.
- `MonthlyRevenue` — return shape for `PaymentRepository::revenue_by_month`.

The two patterns coexist because Pattern B emerged later (every repository added since the original split was added in its own file). Pattern A was the original convention and stuck for the first seven repos.

The `repository-contracts` spec encodes Pattern A as the rule. The newer 5 repos technically violate the spec; the spec is wrong about reality.

## Goals / Non-Goals

**Goals:**
- One canonical pattern: trait + impl + auxiliary types in the per-file module.
- `mod.rs` becomes a module index — `pub mod` declarations + `pub use` re-exports — and nothing else.
- Public import paths (`crate::repository::MemberRepository`, `crate::repository::MonthlyRevenue`, …) are preserved.
- The `repository-contracts` spec accurately describes the file layout.

**Non-Goals:**
- Renaming traits or methods.
- Changing trait surfaces (no new methods, no removed methods, no signature changes).
- Renaming files (`donation_repository.rs` does not become `donation_campaign_repository.rs`, even though it holds `DonationCampaignRepository` — file naming is a separable concern).
- Splitting auxiliary types into their own module (e.g., `repository::query`). They live alongside the trait that uses them; pulling them out adds a hop without gaining clarity since each one is single-use.
- Touching the `Send + Sync` bounds, `async_trait` usage, or `pub trait` visibility. Pattern stays.

## Decisions

### D1. Auxiliary types move with their trait

`MemberQuery`, `MemberSortField`, `SortOrder` move into `member_repository.rs`. `MonthlyRevenue` moves into `payment_repository.rs`. Each auxiliary type is single-use — only the trait that returns/accepts it references it — so co-locating them with the trait is the right call.

Considered: a `repository::query` module collecting all auxiliary types. Rejected — there's no shared theme to motivate the grouping; readers would still have to look at the trait to understand what the type is for. Single-use types don't earn their own home.

### D2. `SortOrder` stays in `member_repository.rs` even though it's a generic-sounding name

Today only `MemberRepository::search` uses `SortOrder`. Co-location with `MemberRepository` is correct now. If a future repository (e.g., `EventRepository::search` if that's ever added) needs the same enum, the right move at that time is to factor it out into a shared location — not pre-emptively. Pre-factoring guesses about future shape; co-location is honest about today's usage.

### D3. `mod.rs` is `pub mod` declarations + `pub use` re-exports, nothing else

```rust
// src/repository/mod.rs (post-change shape)

pub mod member_repository;
pub mod event_repository;
pub mod event_series_repository;
pub mod announcement_repository;
pub mod payment_repository;
pub mod saved_card_repository;
pub mod scheduled_payment_repository;
pub mod donation_repository;
pub mod event_type_repository;
pub mod announcement_type_repository;
pub mod membership_type_repository;
pub mod processed_events_repository;

pub use member_repository::{
    MemberRepository, SqliteMemberRepository,
    MemberQuery, MemberSortField, SortOrder,
};
pub use event_repository::{EventRepository, SqliteEventRepository};
pub use event_series_repository::{EventSeriesRepository, SqliteEventSeriesRepository};
pub use announcement_repository::{AnnouncementRepository, SqliteAnnouncementRepository};
pub use payment_repository::{PaymentRepository, SqlitePaymentRepository, MonthlyRevenue};
pub use saved_card_repository::{SavedCardRepository, SqliteSavedCardRepository};
pub use scheduled_payment_repository::{ScheduledPaymentRepository, SqliteScheduledPaymentRepository};
pub use donation_repository::{DonationCampaignRepository, SqliteDonationCampaignRepository};
pub use event_type_repository::{EventTypeRepository, SqliteEventTypeRepository};
pub use announcement_type_repository::{AnnouncementTypeRepository, SqliteAnnouncementTypeRepository};
pub use membership_type_repository::{MembershipTypeRepository, SqliteMembershipTypeRepository};
pub use processed_events_repository::{ProcessedEventsRepository, SqliteProcessedEventsRepository};
```

The `pub use` block keeps every existing public path resolvable. Callers writing `use crate::repository::MemberRepository` continue to work; callers writing `use crate::repository::member_repository::MemberRepository` also work. We don't need to choose; both stay valid.

### D4. Internal imports inside per-file modules simplify

Today `member_repository.rs` reads:

```rust
use crate::{
    domain::{Member, MemberStatus, ...},
    error::{AppError, Result},
    repository::{MemberQuery, MemberRepository, MemberSortField, SortOrder},
};
```

After the move, `MemberRepository`, `MemberQuery`, `MemberSortField`, `SortOrder` are all in the same file. The import line drops the `repository::{...}` segment entirely:

```rust
use crate::{
    domain::{Member, MemberStatus, ...},
    error::{AppError, Result},
};
```

Same for `payment_repository.rs` (drops `MonthlyRevenue` and `PaymentRepository` from the import). Other per-file modules don't import from `crate::repository::` today, so they're unchanged.

### D5. The `Pattern B` repos already in the per-file shape don't move

`EventSeriesRepository`, `EventTypeRepository`, `AnnouncementTypeRepository`, `MembershipTypeRepository`, `ProcessedEventsRepository` are already correct. The only change for them is that `mod.rs` no longer treats them as "the special ones" — the convention now matches.

### D6. Single PR; ordered changes

The order matters because the compiler needs the full graph to be valid at each step:

1. For each Pattern A trait, copy the trait declaration into the per-file module. Add it alongside the existing impl.
2. Confirm the per-file module's `impl <Trait> for <Sqlite...>` block can resolve the trait it now sits next to. Internal `use crate::repository::{...}` lines go away.
3. Remove the trait declaration from `mod.rs`.
4. Update `mod.rs`'s `pub use` block to re-export the trait from its new home (alongside the impl that's already re-exported).
5. Move `MemberQuery` / `MemberSortField` / `SortOrder` from `mod.rs` to `member_repository.rs`. Update `mod.rs`'s `pub use` to point at the new location.
6. Move `MonthlyRevenue` from `mod.rs` to `payment_repository.rs`. Same re-export update.
7. Confirm `mod.rs` is now ~40 lines and contains only `pub mod` + `pub use`.

Each step ends with a clean compile. The compiler is the regression net.

### D7. No documentation comments lose their home

The trait-level doc comments (e.g., the multi-line doc on `MemberRepository::search` describing the strongly-typed query rationale) move with the trait. Doc comments on auxiliary types (`MemberQuery`, `MonthlyRevenue`) move too. Nothing is lost.

## Risks / Trade-offs

- **Risk**: a future contributor adds a new repository and puts the trait back in `mod.rs` (Pattern A) out of habit. → **Mitigation**: the spec delta makes Pattern B the rule; reviewers can point to the spec if a PR violates it. The codebase becomes uniform, so the rule is also visible by example.
- **Risk**: re-exports drift if a future contributor adds a trait to a module without re-exporting it from `mod.rs`. → **Mitigation**: the existing pattern of re-exporting from `mod.rs` continues; CI builds catch any consumer that imports a no-longer-exported name. Plus a one-line check in code review.
- **Trade-off**: per-file modules grow longer. `member_repository.rs` adds the trait (~80 lines) and the auxiliary types (~30 lines), going from ~584 to ~700 lines. `payment_repository.rs` adds the trait (~80 lines) and `MonthlyRevenue` (~10 lines), going from ~510 to ~600 lines. Both are still single-file comfortable.
- **Trade-off**: callers using `crate::repository::*` (a glob import; one site at `src/service/mod.rs:12`) continue to work because the re-exports cover the same names. No call-site change required.

## Migration Plan

Single PR. Pure file reorganization with compiler-enforced correctness.

1. For each Pattern A trait, in order of low-coupling first (`DonationCampaignRepository`, `SavedCardRepository`, `ScheduledPaymentRepository`, `AnnouncementRepository`, `EventRepository`, `PaymentRepository`, `MemberRepository`):
   - Cut the trait declaration from `mod.rs`.
   - Paste into the matching per-file module just above the `impl <Trait> for <Sqlite...>` block.
   - Drop the now-redundant `repository::<Trait>` from the per-file module's `use crate::{...}` imports.
   - Update `mod.rs`'s `pub use` line to add the trait name.
   - `cargo build` — green.
2. Move auxiliary types:
   - `MemberQuery`, `MemberSortField`, `SortOrder` from `mod.rs` to `member_repository.rs`.
   - `MonthlyRevenue` from `mod.rs` to `payment_repository.rs`.
   - Update `mod.rs`'s `pub use` re-exports to include them.
   - `cargo build` — green.
3. `cargo test --features test-utils`.
4. Eyeball: `wc -l src/repository/mod.rs` should be ~40.
5. Deploy normally. `git revert` is the rollback.
