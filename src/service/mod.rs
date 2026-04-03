pub mod billing_service;
pub mod member_service;
pub mod settings_service;
pub mod event_type_service;
pub mod announcement_type_service;
pub mod membership_type_service;

use std::sync::Arc;
use sqlx::SqlitePool;
use crate::repository::*;
use crate::integrations::IntegrationManager;
use crate::auth::{AuthService, CsrfService};
use settings_service::SettingsService;
use event_type_service::EventTypeService;
use announcement_type_service::AnnouncementTypeService;
use membership_type_service::MembershipTypeService;

pub use membership_type_service::MembershipPricing;

pub struct ServiceContext {
    pub member_repo: Arc<dyn MemberRepository>,
    pub event_repo: Arc<dyn EventRepository>,
    pub announcement_repo: Arc<dyn AnnouncementRepository>,
    pub payment_repo: Arc<dyn PaymentRepository>,
    pub saved_card_repo: Arc<dyn SavedCardRepository>,
    pub scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository>,
    pub integration_manager: Arc<IntegrationManager>,
    pub auth_service: Arc<AuthService>,
    pub csrf_service: Arc<CsrfService>,
    pub settings_service: Arc<SettingsService>,
    pub event_type_service: Arc<EventTypeService>,
    pub announcement_type_service: Arc<AnnouncementTypeService>,
    pub membership_type_service: Arc<MembershipTypeService>,
    pub db_pool: SqlitePool,
}

impl ServiceContext {
    pub fn new(
        member_repo: Arc<dyn MemberRepository>,
        event_repo: Arc<dyn EventRepository>,
        announcement_repo: Arc<dyn AnnouncementRepository>,
        payment_repo: Arc<dyn PaymentRepository>,
        integration_manager: Arc<IntegrationManager>,
        auth_service: Arc<AuthService>,
        db_pool: SqlitePool,
    ) -> Self {
        let settings_service = Arc::new(SettingsService::new(db_pool.clone()));
        let csrf_service = Arc::new(CsrfService::new(db_pool.clone()));

        // Create type repositories
        let event_type_repo = Arc::new(SqliteEventTypeRepository::new(db_pool.clone()));
        let announcement_type_repo = Arc::new(SqliteAnnouncementTypeRepository::new(db_pool.clone()));
        let membership_type_repo = Arc::new(SqliteMembershipTypeRepository::new(db_pool.clone()));

        // Create saved card and scheduled payment repositories
        let saved_card_repo: Arc<dyn SavedCardRepository> = Arc::new(SqliteSavedCardRepository::new(db_pool.clone()));
        let scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository> = Arc::new(SqliteScheduledPaymentRepository::new(db_pool.clone()));

        // Create type services
        let event_type_service = Arc::new(EventTypeService::new(event_type_repo));
        let announcement_type_service = Arc::new(AnnouncementTypeService::new(announcement_type_repo));
        let membership_type_service = Arc::new(MembershipTypeService::new(membership_type_repo));

        Self {
            member_repo,
            event_repo,
            announcement_repo,
            payment_repo,
            saved_card_repo,
            scheduled_payment_repo,
            integration_manager,
            auth_service,
            csrf_service,
            settings_service,
            event_type_service,
            announcement_type_service,
            membership_type_service,
            db_pool,
        }
    }

    /// Create a BillingService from this context.
    pub fn billing_service(
        &self,
        stripe_client: Option<Arc<crate::payments::StripeClient>>,
    ) -> billing_service::BillingService {
        billing_service::BillingService::new(
            self.scheduled_payment_repo.clone(),
            self.payment_repo.clone(),
            self.saved_card_repo.clone(),
            self.membership_type_service.clone(),
            self.settings_service.clone(),
            stripe_client,
            self.db_pool.clone(),
        )
    }
}