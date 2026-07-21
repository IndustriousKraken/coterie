pub mod announcement_admin_service;
pub mod audit_service;
pub mod basic_type_service;
pub mod billing_service;
pub mod configurable_types;
pub mod event_admin_service;
pub mod expense_account_service;
pub mod expense_category_service;
pub mod expense_service;
pub mod member_field_service;
pub mod member_service;
pub mod membership_type_service;
pub mod payment_admin_service;
pub mod payment_service;
pub mod recurring_event_service;
pub mod settings_service;
pub mod stripe_import_service;
pub mod submission_service;
pub mod update_check;

use crate::api::state::MoneyLimiter;
use crate::auth::{AuthService, CsrfService, PendingLoginService, TotpService};
use crate::domain::BasicTypeKind;
use crate::email::EmailSender;
use crate::integrations::IntegrationManager;
use crate::payments::StripeHandle;
use crate::repository::*;
use announcement_admin_service::AnnouncementAdminService;
use audit_service::AuditService;
use basic_type_service::BasicTypeService;
use event_admin_service::EventAdminService;
use expense_account_service::ExpenseAccountService;
use member_field_service::MemberFieldService;
use expense_category_service::ExpenseCategoryService;
use expense_service::ExpenseService;
use member_service::MemberService;
use membership_type_service::MembershipTypeService;
use payment_admin_service::PaymentAdminService;
use payment_service::PaymentService;
use recurring_event_service::RecurringEventService;
use settings_service::SettingsService;
use submission_service::SubmissionService;
use sqlx::SqlitePool;
use std::sync::Arc;

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
    pub basic_type_repo: Arc<dyn BasicTypeRepository>,
    pub membership_type_repo: Arc<dyn MembershipTypeRepository>,
    pub processed_events_repo: Arc<dyn ProcessedEventsRepository>,
    pub expense_repo: Arc<dyn ExpenseRepository>,
    pub expense_category_repo: Arc<dyn ExpenseCategoryRepository>,
    pub expense_account_repo: Arc<dyn ExpenseAccountRepository>,
    pub integration_manager: Arc<IntegrationManager>,
    pub auth_service: Arc<AuthService>,
    pub csrf_service: Arc<CsrfService>,
    pub totp_service: Arc<TotpService>,
    pub pending_login_service: Arc<PendingLoginService>,
    pub settings_service: Arc<SettingsService>,
    pub event_type_service: Arc<BasicTypeService>,
    pub announcement_type_service: Arc<BasicTypeService>,
    pub membership_type_service: Arc<MembershipTypeService>,
    pub email_sender: Arc<dyn EmailSender>,
    pub audit_service: Arc<AuditService>,
    pub payment_service: Arc<PaymentService>,
    pub member_service: Arc<MemberService>,
    pub event_admin_service: Arc<EventAdminService>,
    pub announcement_admin_service: Arc<AnnouncementAdminService>,
    pub payment_admin_service: Arc<PaymentAdminService>,
    /// Hot-swappable Stripe wiring (client + inbound webhook dispatcher),
    /// rebuilt from DB config on a portal settings save. Owned here so the
    /// background services (auto-renew, refunds) and the per-request /
    /// webhook paths all read the SAME handle — a key rotation reaches
    /// every path without a restart. Built empty; `rebuild()` at startup
    /// loads DB config.
    pub stripe_handle: Arc<StripeHandle>,
    pub member_field_repo: Arc<dyn MemberFieldRepository>,
    pub member_field_service: Arc<MemberFieldService>,
    pub expense_service: Arc<ExpenseService>,
    pub expense_category_service: Arc<ExpenseCategoryService>,
    pub expense_account_service: Arc<ExpenseAccountService>,
    pub submission_repo: Arc<dyn SubmissionRepository>,
    pub submission_service: Arc<SubmissionService>,
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
        money_limiter: MoneyLimiter,
        base_url: String,
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

        // Create type repositories. One basic-type repo serves both event
        // and announcement kinds; membership types stay separate.
        let basic_type_repo: Arc<dyn BasicTypeRepository> =
            Arc::new(SqliteBasicTypeRepository::new(db_pool.clone()));
        let membership_type_repo: Arc<dyn MembershipTypeRepository> =
            Arc::new(SqliteMembershipTypeRepository::new(db_pool.clone()));
        let processed_events_repo: Arc<dyn ProcessedEventsRepository> =
            Arc::new(SqliteProcessedEventsRepository::new(db_pool.clone()));

        // Create saved card and scheduled payment repositories
        let saved_card_repo: Arc<dyn SavedCardRepository> =
            Arc::new(SqliteSavedCardRepository::new(db_pool.clone()));
        let scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository> =
            Arc::new(SqliteScheduledPaymentRepository::new(db_pool.clone()));
        let donation_campaign_repo: Arc<dyn DonationCampaignRepository> =
            Arc::new(SqliteDonationCampaignRepository::new(db_pool.clone()));

        // Expense ledger repositories — see expense-tracking capability.
        let expense_repo: Arc<dyn ExpenseRepository> =
            Arc::new(SqliteExpenseRepository::new(db_pool.clone()));
        let expense_category_repo: Arc<dyn ExpenseCategoryRepository> =
            Arc::new(SqliteExpenseCategoryRepository::new(db_pool.clone()));
        let expense_account_repo: Arc<dyn ExpenseAccountRepository> =
            Arc::new(SqliteExpenseAccountRepository::new(db_pool.clone()));

        // Create type services. Two BasicTypeService instances share the
        // basic-type repo Arc; each bakes in its own BasicTypeKind so call
        // sites stay unchanged.
        let event_type_service = Arc::new(BasicTypeService::new(
            basic_type_repo.clone(),
            BasicTypeKind::Event,
        ));
        let announcement_type_service = Arc::new(BasicTypeService::new(
            basic_type_repo.clone(),
            BasicTypeKind::Announcement,
        ));
        let membership_type_service =
            Arc::new(MembershipTypeService::new(membership_type_repo.clone()));

        // Single hot-swappable Stripe handle, shared by the background
        // services below (refunds, auto-renew) AND the per-request /
        // webhook paths (via AppState). Built empty; the caller runs
        // `rebuild()` once at startup to load DB config, and the portal
        // settings save calls `rebuild()` again so a key rotation reaches
        // every path with no restart. All deps it needs exist by now.
        let stripe_handle = Arc::new(StripeHandle::new(
            settings_service.clone(),
            payment_repo.clone(),
            member_repo.clone(),
            processed_events_repo.clone(),
            membership_type_service.clone(),
            integration_manager.clone(),
        ));

        let payment_service = Arc::new(PaymentService::new(
            payment_repo.clone(),
            member_repo.clone(),
            donation_campaign_repo.clone(),
            audit_service.clone(),
        ));

        let member_service = Arc::new(MemberService::new(
            member_repo.clone(),
            auth_service.clone(),
            audit_service.clone(),
            integration_manager.clone(),
            email_sender.clone(),
            membership_type_service.clone(),
            settings_service.clone(),
            db_pool.clone(),
            base_url,
        ));

        let event_admin_service = Arc::new(EventAdminService::new(
            event_repo.clone(),
            event_series_repo.clone(),
            recurring_event_service.clone(),
            audit_service.clone(),
            integration_manager.clone(),
        ));

        let announcement_admin_service = Arc::new(AnnouncementAdminService::new(
            announcement_repo.clone(),
            audit_service.clone(),
            integration_manager.clone(),
        ));

        let payment_admin_service = Arc::new(PaymentAdminService::new(
            payment_repo.clone(),
            stripe_handle.clone(),
            audit_service.clone(),
            integration_manager.clone(),
            money_limiter,
        ));

        // Org-defined member fields — see member-custom-fields capability.
        let member_field_repo: Arc<dyn MemberFieldRepository> =
            Arc::new(SqliteMemberFieldRepository::new(db_pool.clone()));
        let member_field_service = Arc::new(MemberFieldService::new(
            member_field_repo.clone(),
            audit_service.clone(),
        ));

        let expense_service = Arc::new(ExpenseService::new(
            expense_repo.clone(),
            expense_category_repo.clone(),
            expense_account_repo.clone(),
            audit_service.clone(),
        ));
        let expense_category_service = Arc::new(ExpenseCategoryService::new(
            expense_category_repo.clone(),
            audit_service.clone(),
        ));
        let expense_account_service = Arc::new(ExpenseAccountService::new(
            expense_account_repo.clone(),
            audit_service.clone(),
        ));

        // Member proposal submissions. Promotion reuses `event_admin_service`
        // (built above) so an accepted+scheduled proposal becomes a standard
        // Event through the existing audited path, not a second event surface.
        let submission_repo: Arc<dyn SubmissionRepository> =
            Arc::new(SqliteSubmissionRepository::new(db_pool.clone()));
        let submission_service = Arc::new(SubmissionService::new(
            submission_repo.clone(),
            event_admin_service.clone(),
            audit_service.clone(),
        ));

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
            basic_type_repo,
            membership_type_repo,
            processed_events_repo,
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
            payment_service,
            member_service,
            event_admin_service,
            announcement_admin_service,
            payment_admin_service,
            stripe_handle,
            expense_repo,
            expense_category_repo,
            expense_account_repo,
            member_field_repo,
            member_field_service,
            expense_service,
            expense_category_service,
            expense_account_service,
            submission_repo,
            submission_service,
            db_pool,
        }
    }

    /// Build the singleton BillingService for this app instance.
    /// Called once at startup; the resulting `Arc<BillingService>`
    /// is stored on `AppState` and shared by every handler. Was a
    /// per-request factory before — see the BillingService field
    /// doc on AppState for why.
    pub fn billing_service(&self, base_url: String) -> billing_service::BillingService {
        billing_service::BillingService::new(
            self.scheduled_payment_repo.clone(),
            self.payment_repo.clone(),
            self.saved_card_repo.clone(),
            self.member_repo.clone(),
            self.event_repo.clone(),
            self.membership_type_service.clone(),
            self.settings_service.clone(),
            self.email_sender.clone(),
            self.integration_manager.clone(),
            self.stripe_handle.clone(),
            base_url,
            self.db_pool.clone(),
        )
    }
}
