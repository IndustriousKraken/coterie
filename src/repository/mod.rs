use async_trait::async_trait;
use uuid::Uuid;
use crate::domain::*;
use crate::error::Result;

pub mod member_repository;
pub mod event_repository;
pub mod announcement_repository;
pub mod payment_repository;
pub mod saved_card_repository;
pub mod scheduled_payment_repository;
pub mod donation_repository;
pub mod event_type_repository;
pub mod announcement_type_repository;
pub mod membership_type_repository;

pub use member_repository::SqliteMemberRepository;
pub use event_repository::SqliteEventRepository;
pub use announcement_repository::SqliteAnnouncementRepository;
pub use payment_repository::SqlitePaymentRepository;
pub use saved_card_repository::SqliteSavedCardRepository;
pub use scheduled_payment_repository::SqliteScheduledPaymentRepository;
pub use donation_repository::SqliteDonationCampaignRepository;
pub use event_type_repository::{EventTypeRepository, SqliteEventTypeRepository};
pub use announcement_type_repository::{AnnouncementTypeRepository, SqliteAnnouncementTypeRepository};
pub use membership_type_repository::{MembershipTypeRepository, SqliteMembershipTypeRepository};

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
}

#[async_trait]
pub trait DonationCampaignRepository: Send + Sync {
    async fn create(&self, campaign: DonationCampaign) -> Result<DonationCampaign>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<DonationCampaign>>;
    async fn find_by_slug(&self, slug: &str) -> Result<Option<DonationCampaign>>;
    async fn list_active(&self) -> Result<Vec<DonationCampaign>>;
    async fn get_total_donated(&self, campaign_id: Uuid) -> Result<i64>;
}