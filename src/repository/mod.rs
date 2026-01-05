use async_trait::async_trait;
use uuid::Uuid;
use crate::domain::*;
use crate::error::Result;

pub mod member_repository;
pub mod event_repository;
pub mod announcement_repository;
pub mod payment_repository;
pub mod event_type_repository;
pub mod announcement_type_repository;
pub mod membership_type_repository;

pub use member_repository::SqliteMemberRepository;
pub use event_repository::SqliteEventRepository;
pub use announcement_repository::SqliteAnnouncementRepository;
pub use payment_repository::SqlitePaymentRepository;
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
    async fn update(&self, id: Uuid, update: UpdateMemberRequest) -> Result<Member>;
    async fn delete(&self, id: Uuid) -> Result<()>;
}

#[async_trait]
pub trait EventRepository: Send + Sync {
    async fn create(&self, event: Event) -> Result<Event>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Event>>;
    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Event>>;
    async fn list_upcoming(&self, limit: i64) -> Result<Vec<Event>>;
    async fn list_public(&self) -> Result<Vec<Event>>;
    async fn update(&self, id: Uuid, event: Event) -> Result<Event>;
    async fn delete(&self, id: Uuid) -> Result<()>;
    async fn register_attendance(&self, event_id: Uuid, member_id: Uuid) -> Result<()>;
    async fn cancel_attendance(&self, event_id: Uuid, member_id: Uuid) -> Result<()>;
    async fn get_attendee_count(&self, event_id: Uuid) -> Result<i64>;
}

#[async_trait]
pub trait AnnouncementRepository: Send + Sync {
    async fn create(&self, announcement: Announcement) -> Result<Announcement>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Announcement>>;
    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Announcement>>;
    async fn list_recent(&self, limit: i64) -> Result<Vec<Announcement>>;
    async fn list_public(&self) -> Result<Vec<Announcement>>;
    async fn list_featured(&self) -> Result<Vec<Announcement>>;
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
}