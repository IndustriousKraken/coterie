use async_trait::async_trait;
use uuid::Uuid;
use crate::domain::*;
use crate::error::Result;

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

pub use member_repository::SqliteMemberRepository;
pub use event_repository::SqliteEventRepository;
pub use event_series_repository::{EventSeriesRepository, SqliteEventSeriesRepository};
pub use announcement_repository::SqliteAnnouncementRepository;
pub use payment_repository::SqlitePaymentRepository;
pub use saved_card_repository::SqliteSavedCardRepository;
pub use scheduled_payment_repository::SqliteScheduledPaymentRepository;
pub use donation_repository::SqliteDonationCampaignRepository;
pub use event_type_repository::{EventTypeRepository, SqliteEventTypeRepository};
pub use announcement_type_repository::{AnnouncementTypeRepository, SqliteAnnouncementTypeRepository};
pub use membership_type_repository::{MembershipTypeRepository, SqliteMembershipTypeRepository};
pub use processed_events_repository::{ProcessedEventsRepository, SqliteProcessedEventsRepository};

#[async_trait]
pub trait MemberRepository: Send + Sync {
    async fn create(&self, member: CreateMemberRequest) -> Result<Member>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Member>>;
    async fn find_by_email(&self, email: &str) -> Result<Option<Member>>;
    async fn find_by_username(&self, username: &str) -> Result<Option<Member>>;
    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Member>>;
    async fn list_active(&self) -> Result<Vec<Member>>;
    async fn list_expired(&self) -> Result<Vec<Member>>;
    /// Every member with a non-empty `discord_id`, regardless of
    /// status. Used by the Discord reconcile sweep so we can catch
    /// drift on Active / Honorary / Expired / Suspended members in
    /// one pass.
    async fn list_with_discord_id(&self) -> Result<Vec<Member>>;
    async fn update(&self, id: Uuid, update: UpdateMemberRequest) -> Result<Member>;
    async fn set_admin(&self, id: Uuid, is_admin: bool) -> Result<Member>;
    async fn mark_email_verified(&self, id: Uuid) -> Result<()>;
    async fn update_password_hash(&self, id: Uuid, password_hash: &str) -> Result<()>;
    /// Set or clear the member's Discord snowflake ID. `None` clears it.
    /// Validation is the caller's responsibility (see
    /// `integrations::discord::is_valid_snowflake`).
    async fn update_discord_id(&self, id: Uuid, discord_id: Option<&str>) -> Result<()>;
    /// Set the member's `dues_paid_until`, revive Expired→Active in
    /// the same UPDATE, and clear the dues-reminder flag so the next
    /// dues cycle can re-fire a reminder. Suspended/Honorary/Pending
    /// statuses are left untouched. This is the single source of
    /// truth for "a payment was just recorded" — every membership
    /// payment path goes through here.
    async fn set_dues_paid_until_with_revival(
        &self,
        id: Uuid,
        new_dues_paid_until: chrono::DateTime<chrono::Utc>,
    ) -> Result<()>;
    /// Inverse of `set_dues_paid_until_with_revival`: backdate
    /// `dues_paid_until` to yesterday and flip Active→Expired in the
    /// same UPDATE. Pending/Suspended/Honorary are left alone (same
    /// asymmetric carve-outs as revival). Used by the admin "expire
    /// now" action; the billing runner would do this on its next tick
    /// anyway, but admins reasonably expect the change to be live
    /// immediately.
    async fn expire_dues_now(&self, id: Uuid) -> Result<()>;
    /// Stamp `dues_reminder_sent_at = CURRENT_TIMESTAMP`. Called from
    /// the dues-reminder runner once the email has gone out, so the
    /// next sweep won't re-send for this dues cycle. Cleared on
    /// payment via `set_dues_paid_until_with_revival`.
    async fn set_dues_reminder_sent(&self, id: Uuid) -> Result<()>;
    /// Update billing mode and subscription id atomically. Pass
    /// `Some(&id)` to set the Stripe subscription id, `None` to clear
    /// it (the right move when leaving `StripeSubscription`). Used by
    /// the auto-renew lifecycle in `BillingService` and by the
    /// Stripe webhook handler when a subscription gets cancelled
    /// out-of-band.
    async fn set_billing_mode(
        &self,
        id: Uuid,
        mode: BillingMode,
        stripe_subscription_id: Option<&str>,
    ) -> Result<()>;
    /// Persist the Stripe customer id for a member. Customer ids are
    /// created lazily on first charge / SetupIntent so this gets
    /// called exactly once per member's lifetime.
    async fn set_stripe_customer_id(&self, id: Uuid, customer_id: &str) -> Result<()>;
    /// Reverse of `stripe_customer_id` — given the Stripe-side id,
    /// find the Coterie member. The webhook handlers use this to
    /// route Stripe events back onto the right row.
    async fn find_by_stripe_customer_id(&self, customer_id: &str) -> Result<Option<Member>>;
    /// Count of members currently in a given billing mode. Drives
    /// the admin "Stripe-sub members remaining" badge.
    async fn count_by_billing_mode(&self, mode: BillingMode) -> Result<i64>;
    /// Member ids in a given billing mode. Used by the bulk-migrate
    /// job that flips every `stripe_subscription` member to
    /// `coterie_managed`.
    async fn list_ids_by_billing_mode(&self, mode: BillingMode) -> Result<Vec<Uuid>>;
    async fn delete(&self, id: Uuid) -> Result<()>;
}

#[async_trait]
pub trait EventRepository: Send + Sync {
    async fn create(&self, event: Event) -> Result<Event>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Event>>;
    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Event>>;
    async fn list_upcoming(&self, limit: i64) -> Result<Vec<Event>>;
    async fn list_public(&self) -> Result<Vec<Event>>;
    async fn list_members_only(&self) -> Result<Vec<Event>>;
    async fn count_members_only_upcoming(&self) -> Result<i64>;
    async fn update(&self, id: Uuid, event: Event) -> Result<Event>;
    async fn delete(&self, id: Uuid) -> Result<()>;
    async fn register_attendance(&self, event_id: Uuid, member_id: Uuid) -> Result<()>;
    async fn cancel_attendance(&self, event_id: Uuid, member_id: Uuid) -> Result<()>;
    async fn get_attendee_count(&self, event_id: Uuid) -> Result<i64>;
    async fn get_member_attendance_status(&self, event_id: Uuid, member_id: Uuid) -> Result<Option<AttendanceStatus>>;
    async fn get_member_registered_events(&self, member_id: Uuid) -> Result<Vec<Event>>;

    // ---- Recurring-series support -------------------------------------

    /// Highest `occurrence_index` already materialized for this series,
    /// or `None` if the series has no rows yet. Used by the materializer
    /// to continue numbering on horizon-extension passes.
    async fn max_occurrence_index_for_series(&self, series_id: Uuid) -> Result<Option<i32>>;
    /// Hard-delete every occurrence in the series whose `start_time`
    /// is strictly greater than `after`. Returns the count deleted.
    /// Used by "end the series after this date" and by the
    /// re-materialization safety net.
    async fn delete_series_occurrences_after(
        &self,
        series_id: Uuid,
        after: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64>;
    /// Apply the editable subset of fields (title, description, type,
    /// visibility, location, max_attendees, rsvp_required) to every
    /// occurrence in the series whose `start_time >= from`. Used by
    /// the "edit this and all future" admin action — start_time and
    /// per-row image_url are deliberately preserved per occurrence.
    async fn update_series_occurrences_from(
        &self,
        series_id: Uuid,
        from: chrono::DateTime<chrono::Utc>,
        template: &Event,
    ) -> Result<u64>;
}

#[async_trait]
pub trait AnnouncementRepository: Send + Sync {
    async fn create(&self, announcement: Announcement) -> Result<Announcement>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Announcement>>;
    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Announcement>>;
    async fn list_recent(&self, limit: i64) -> Result<Vec<Announcement>>;
    async fn list_public(&self) -> Result<Vec<Announcement>>;
    async fn list_featured(&self) -> Result<Vec<Announcement>>;
    async fn count_private_published(&self) -> Result<i64>;
    async fn update(&self, id: Uuid, announcement: Announcement) -> Result<Announcement>;
    async fn delete(&self, id: Uuid) -> Result<()>;
}

#[async_trait]
pub trait PaymentRepository: Send + Sync {
    async fn create(&self, payment: Payment) -> Result<Payment>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Payment>>;
    async fn find_by_member(&self, member_id: Uuid) -> Result<Vec<Payment>>;
    async fn find_by_stripe_id(&self, stripe_id: &str) -> Result<Option<Payment>>;
    async fn update(&self, id: Uuid, payment: Payment) -> Result<Payment>;
    async fn update_status(&self, id: Uuid, status: PaymentStatus) -> Result<Payment>;
    /// Atomically flip a Pending payment to Completed and stamp the
    /// Stripe PaymentIntent ID. Returns `true` if the row was actually
    /// flipped (we own the post-payment work — extend dues, schedule
    /// next renewal); `false` if the row had already been completed by
    /// another caller (sync path vs. webhook race). The semantics
    /// guarantee that exactly one caller does the post-work.
    async fn complete_pending_payment(
        &self,
        id: Uuid,
        stripe_payment_id: &str,
    ) -> Result<bool>;
    /// Counterpart to `complete_pending_payment` for the failure path:
    /// flip a Pending row to Failed when the Stripe charge errored.
    /// Returns true if a row was flipped. Idempotent against double-
    /// failure reports.
    async fn fail_pending_payment(&self, id: Uuid) -> Result<bool>;
    /// Claim a Completed payment for refund. Atomic conditional UPDATE
    /// (`WHERE status='Completed'`) — only the first caller observes
    /// rows_affected==1; concurrent admin clicks see false and bail.
    /// Pair with `unclaim_refund` if the subsequent Stripe call fails.
    async fn claim_payment_for_refund(&self, id: Uuid) -> Result<bool>;
    /// Roll back `claim_payment_for_refund` after a Stripe failure so
    /// the row goes back to Completed and a future refund attempt can
    /// re-claim. Conditional on status='Refunded' so this can't undo
    /// a legitimate completed refund from a different code path.
    async fn unclaim_refund(&self, id: Uuid) -> Result<()>;
    /// Mark a payment Refunded, unconditionally. Used by the Stripe
    /// `charge.refunded` webhook handler when the row hasn't already
    /// been flipped by our own admin-button refund (caller filters
    /// out `Refunded` echoes). Idempotent under repeat calls.
    async fn mark_refunded(&self, id: Uuid) -> Result<()>;
    /// Idempotently extend a member's dues for a single Payment.
    ///
    /// Implemented as a transactional claim-then-update: the row's
    /// `dues_extended_at` is set to NOW under a per-payment uniqueness
    /// guard, and `dues_paid_until` is recomputed from the latest
    /// member state (read inside the same transaction so concurrent
    /// payments can't lose each other's increments). Returns `true`
    /// if THIS call did the extension; `false` if a previous call
    /// already extended dues for this payment.
    ///
    /// This single method addresses two correctness issues:
    /// (1) Stripe webhook retries that re-run a handler after a
    ///     transient failure no longer double-extend dues (the second
    ///     call sees the claim and no-ops).
    /// (2) Two payments for the same member processed concurrently
    ///     can't both compute `D + 1y` from the same starting `D` —
    ///     the SQLite write lock serializes the SELECT/UPDATE pair.
    async fn extend_dues_for_payment_atomic(
        &self,
        payment_id: Uuid,
        member_id: Uuid,
        billing_period: crate::domain::configurable_types::BillingPeriod,
    ) -> Result<bool>;

    // ---- Admin billing dashboard support ------------------------------

    /// Sum of completed-payment cents grouped by (year, month,
    /// payment_type) across the last `months_back` months of `paid_at`.
    /// Refunded / Pending / Failed rows are excluded — they'd mislead
    /// "what we actually collected." Ordered newest month first.
    async fn revenue_by_month(&self, months_back: u32) -> Result<Vec<MonthlyRevenue>>;
}

/// Single (month, payment_type) bucket for the admin billing dashboard.
/// `payment_type` is the raw lowercase DB-column value
/// (`"membership" | "donation" | "other"`) — this is a SQL aggregation
/// row, not a real `Payment`, so we don't try to lift it into the
/// richer `PaymentKind` (Donation needs a campaign id we don't carry
/// at the bucket level). Callers match on the string.
#[derive(Debug, Clone)]
pub struct MonthlyRevenue {
    pub year: i32,
    pub month: u32,
    pub payment_type: String,
    pub total_cents: i64,
    pub payment_count: i64,
}

#[async_trait]
pub trait SavedCardRepository: Send + Sync {
    async fn create(&self, card: SavedCard) -> Result<SavedCard>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<SavedCard>>;
    async fn find_by_member(&self, member_id: Uuid) -> Result<Vec<SavedCard>>;
    async fn find_default_for_member(&self, member_id: Uuid) -> Result<Option<SavedCard>>;
    async fn set_default(&self, member_id: Uuid, card_id: Uuid) -> Result<()>;
    async fn delete(&self, id: Uuid) -> Result<()>;
}

#[async_trait]
pub trait ScheduledPaymentRepository: Send + Sync {
    async fn create(&self, payment: ScheduledPayment) -> Result<ScheduledPayment>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<ScheduledPayment>>;
    async fn find_by_member(&self, member_id: Uuid) -> Result<Vec<ScheduledPayment>>;
    async fn find_pending_due_before(&self, date: chrono::NaiveDate) -> Result<Vec<ScheduledPayment>>;
    async fn update_status(&self, id: Uuid, status: ScheduledPaymentStatus, failure_reason: Option<String>) -> Result<ScheduledPayment>;
    async fn increment_retry(&self, id: Uuid) -> Result<ScheduledPayment>;
    async fn link_payment(&self, id: Uuid, payment_id: Uuid) -> Result<ScheduledPayment>;
    /// Failed scheduled payments whose last attempt landed in
    /// `[since, now]`. The admin billing dashboard surfaces these
    /// alongside their retry_count + failure_reason so an operator
    /// can see "what's piling up." Ordered newest-attempt first.
    async fn list_failures_since(
        &self,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<ScheduledPayment>>;
}

#[async_trait]
pub trait DonationCampaignRepository: Send + Sync {
    async fn create(&self, campaign: DonationCampaign) -> Result<DonationCampaign>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<DonationCampaign>>;
    async fn find_by_slug(&self, slug: &str) -> Result<Option<DonationCampaign>>;
    async fn list_active(&self) -> Result<Vec<DonationCampaign>>;
    async fn get_total_donated(&self, campaign_id: Uuid) -> Result<i64>;
}