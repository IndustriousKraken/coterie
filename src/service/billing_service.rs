//! Container over three independently-testable sub-services:
//! [`auto_renew::AutoRenew`], [`notifications::Notifications`], and
//! [`expiration::Expiration`]. Splitting the original 1300-line
//! `BillingService` along these lines means each sub-module has a
//! single concern and a small, obviously-correct dependency set.
//!
//! Callers reach sub-service methods via direct field access
//! (`billing_service.auto_renew.run_billing_cycle()`), so the
//! structural grouping is legible at every call site.

pub mod auto_renew;
pub mod expiration;
pub mod notifications;

use sqlx::SqlitePool;
use std::sync::Arc;

use crate::{
    email::EmailSender,
    integrations::IntegrationManager,
    payments::StripeClient,
    repository::{
        EventRepository, MemberRepository, PaymentRepository, SavedCardRepository,
        ScheduledPaymentRepository,
    },
    service::{membership_type_service::MembershipTypeService, settings_service::SettingsService},
};

pub struct BillingService {
    pub auto_renew: auto_renew::AutoRenew,
    pub notifications: notifications::Notifications,
    pub expiration: expiration::Expiration,
}

impl BillingService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository>,
        payment_repo: Arc<dyn PaymentRepository>,
        saved_card_repo: Arc<dyn SavedCardRepository>,
        member_repo: Arc<dyn MemberRepository>,
        event_repo: Arc<dyn EventRepository>,
        membership_type_service: Arc<MembershipTypeService>,
        settings_service: Arc<SettingsService>,
        email_sender: Arc<dyn EmailSender>,
        integration_manager: Arc<IntegrationManager>,
        stripe_client: Option<Arc<StripeClient>>,
        base_url: String,
        db_pool: SqlitePool,
    ) -> Self {
        let auto_renew = auto_renew::AutoRenew::new(
            scheduled_payment_repo,
            payment_repo,
            saved_card_repo.clone(),
            member_repo.clone(),
            membership_type_service.clone(),
            settings_service.clone(),
            integration_manager.clone(),
            stripe_client,
            base_url.clone(),
        );
        let notifications = notifications::Notifications::new(
            member_repo.clone(),
            saved_card_repo,
            event_repo,
            membership_type_service,
            settings_service.clone(),
            email_sender,
            integration_manager.clone(),
            base_url,
            db_pool.clone(),
        );
        let expiration = expiration::Expiration::new(
            member_repo,
            settings_service,
            integration_manager,
            db_pool,
        );
        Self {
            auto_renew,
            notifications,
            expiration,
        }
    }
}
