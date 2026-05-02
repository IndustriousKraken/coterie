//! Thin facade over three independently-testable sub-services:
//! [`auto_renew::AutoRenew`], [`notifications::Notifications`], and
//! [`expiration::Expiration`]. Splitting the original 1300-line
//! `BillingService` along these lines means each sub-module has a
//! single concern and a small, obviously-correct dependency set.
//!
//! The facade exists so callers don't have to know about the split —
//! every method on the original `BillingService` still resolves at
//! the same path.

mod auto_renew;
mod expiration;
mod notifications;

use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    email::EmailSender,
    error::Result,
    integrations::IntegrationManager,
    payments::StripeClient,
    repository::{
        MemberRepository, PaymentRepository, SavedCardRepository, ScheduledPaymentRepository,
    },
    service::{membership_type_service::MembershipTypeService, settings_service::SettingsService},
};

pub use auto_renew::BulkMigrationSummary;

pub struct BillingService {
    auto_renew: auto_renew::AutoRenew,
    notifications: notifications::Notifications,
    expiration: expiration::Expiration,
}

impl BillingService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository>,
        payment_repo: Arc<dyn PaymentRepository>,
        saved_card_repo: Arc<dyn SavedCardRepository>,
        member_repo: Arc<dyn MemberRepository>,
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
        );
        let notifications = notifications::Notifications::new(
            member_repo.clone(),
            saved_card_repo,
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
        Self { auto_renew, notifications, expiration }
    }

    // ---- Auto-renew lifecycle + charge runner -------------------------

    pub async fn migrate_to_coterie_managed(&self, member_id: Uuid) -> Result<bool> {
        self.auto_renew.migrate_to_coterie_managed(member_id).await
    }

    pub async fn bulk_migrate_stripe_subscriptions(&self) -> BulkMigrationSummary {
        self.auto_renew.bulk_migrate_stripe_subscriptions().await
    }

    pub async fn enable_auto_renew(
        &self,
        member_id: Uuid,
        membership_type_slug: &str,
    ) -> Result<()> {
        self.auto_renew.enable_auto_renew(member_id, membership_type_slug).await
    }

    pub async fn reschedule_after_payment(
        &self,
        member_id: Uuid,
        membership_type_slug: &str,
    ) -> Result<()> {
        self.auto_renew.reschedule_after_payment(member_id, membership_type_slug).await
    }

    pub async fn disable_auto_renew(&self, member_id: Uuid) -> Result<()> {
        self.auto_renew.disable_auto_renew(member_id).await
    }

    pub async fn run_billing_cycle(&self) -> Result<(u32, u32)> {
        self.auto_renew.run_billing_cycle().await
    }

    pub async fn extend_member_dues_by_slug(
        &self,
        payment_id: Uuid,
        member_id: Uuid,
        membership_type_slug: &str,
    ) -> Result<()> {
        self.auto_renew.extend_member_dues_by_slug(payment_id, member_id, membership_type_slug).await
    }

    // ---- Notifications -----------------------------------------------

    pub async fn notify_subscription_cancelled(&self, member_id: Uuid) -> Result<()> {
        self.notifications.notify_subscription_cancelled(member_id).await
    }

    pub async fn notify_subscription_payment_failed(
        &self,
        member_id: Uuid,
        amount_display: Option<String>,
        is_final: bool,
    ) -> Result<()> {
        self.notifications.notify_subscription_payment_failed(member_id, amount_display, is_final).await
    }

    pub async fn send_dues_reminders(&self) -> Result<u32> {
        self.notifications.send_dues_reminders().await
    }

    // ---- Expiration sweep --------------------------------------------

    pub async fn check_expired_members(&self) -> Result<u32> {
        self.expiration.check_expired_members().await
    }
}
