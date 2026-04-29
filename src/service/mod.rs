pub mod audit_service;
pub mod billing_service;
pub mod member_service;
pub mod recurring_event_service;
pub mod settings_service;
pub mod event_type_service;
pub mod announcement_type_service;
pub mod membership_type_service;

use std::sync::Arc;
use sqlx::SqlitePool;
use crate::repository::*;
use crate::integrations::IntegrationManager;
use crate::auth::{AuthService, CsrfService, PendingLoginService, TotpService};
use crate::email::EmailSender;
use audit_service::AuditService;
use settings_service::SettingsService;
use event_type_service::EventTypeService;
use announcement_type_service::AnnouncementTypeService;
use membership_type_service::MembershipTypeService;
use recurring_event_service::RecurringEventService;

pub use membership_type_service::MembershipPricing;

pub struct ServiceContext {
    pub member_repo: Arc<dyn MemberRepository>,
    pub event_repo: Arc<dyn EventRepository>,
    pub event_series_repo: Arc<dyn EventSeriesRepository>,
    pub recurring_event_service: Arc<RecurringEventService>,
    pub announcement_repo: Arc<dyn AnnouncementRepository>,
    pub payment_repo: Arc<dyn PaymentRepository>,
    pub saved_card_repo: Arc<dyn SavedCardRepository>,
    pub scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository>,
    pub donation_campaign_repo: Arc<dyn DonationCampaignRepository>,
    pub integration_manager: Arc<IntegrationManager>,
    pub auth_service: Arc<AuthService>,
    pub csrf_service: Arc<CsrfService>,
    pub totp_service: Arc<TotpService>,
    pub pending_login_service: Arc<PendingLoginService>,
    pub settings_service: Arc<SettingsService>,
    pub event_type_service: Arc<EventTypeService>,
    pub announcement_type_service: Arc<AnnouncementTypeService>,
    pub membership_type_service: Arc<MembershipTypeService>,
    pub email_sender: Arc<dyn EmailSender>,
    pub audit_service: Arc<AuditService>,
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
        email_sender: Arc<dyn EmailSender>,
        settings_service: Arc<SettingsService>,
        csrf_service: Arc<CsrfService>,
        totp_service: Arc<TotpService>,
        pending_login_service: Arc<PendingLoginService>,
        db_pool: SqlitePool,
    ) -> Self {
        let event_series_repo: Arc<dyn EventSeriesRepository> =
            Arc::new(SqliteEventSeriesRepository::new(db_pool.clone()));
        let recurring_event_service = Arc::new(RecurringEventService::new(
            event_repo.clone(),
            event_series_repo.clone(),
            db_pool.clone(),
        ));
        let audit_service = Arc::new(AuditService::new(db_pool.clone()));

        // Create type repositories
        let event_type_repo = Arc::new(SqliteEventTypeRepository::new(db_pool.clone()));
        let announcement_type_repo = Arc::new(SqliteAnnouncementTypeRepository::new(db_pool.clone()));
        let membership_type_repo = Arc::new(SqliteMembershipTypeRepository::new(db_pool.clone()));

        // Create saved card and scheduled payment repositories
        let saved_card_repo: Arc<dyn SavedCardRepository> = Arc::new(SqliteSavedCardRepository::new(db_pool.clone()));
        let scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository> = Arc::new(SqliteScheduledPaymentRepository::new(db_pool.clone()));
        let donation_campaign_repo: Arc<dyn DonationCampaignRepository> = Arc::new(SqliteDonationCampaignRepository::new(db_pool.clone()));

        // Create type services
        let event_type_service = Arc::new(EventTypeService::new(event_type_repo));
        let announcement_type_service = Arc::new(AnnouncementTypeService::new(announcement_type_repo));
        let membership_type_service = Arc::new(MembershipTypeService::new(membership_type_repo));

        Self {
            member_repo,
            event_repo,
            event_series_repo,
            recurring_event_service,
            announcement_repo,
            payment_repo,
            saved_card_repo,
            scheduled_payment_repo,
            donation_campaign_repo,
            integration_manager,
            auth_service,
            csrf_service,
            totp_service,
            pending_login_service,
            settings_service,
            event_type_service,
            announcement_type_service,
            membership_type_service,
            email_sender,
            audit_service,
            db_pool,
        }
    }

    /// Create a BillingService from this context.
    pub fn billing_service(
        &self,
        stripe_client: Option<Arc<crate::payments::StripeClient>>,
        base_url: String,
    ) -> billing_service::BillingService {
        billing_service::BillingService::new(
            self.scheduled_payment_repo.clone(),
            self.payment_repo.clone(),
            self.saved_card_repo.clone(),
            self.member_repo.clone(),
            self.membership_type_service.clone(),
            self.settings_service.clone(),
            self.email_sender.clone(),
            self.integration_manager.clone(),
            stripe_client,
            base_url,
            self.db_pool.clone(),
        )
    }
}