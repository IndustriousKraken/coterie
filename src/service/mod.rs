pub mod member_service;

use std::sync::Arc;
use sqlx::SqlitePool;
use crate::repository::*;
use crate::integrations::IntegrationManager;
use crate::auth::AuthService;

pub struct ServiceContext {
    pub member_repo: Arc<dyn MemberRepository>,
    pub event_repo: Arc<dyn EventRepository>,
    pub announcement_repo: Arc<dyn AnnouncementRepository>,
    pub payment_repo: Arc<dyn PaymentRepository>,
    pub integration_manager: Arc<IntegrationManager>,
    pub auth_service: Arc<AuthService>,
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
        Self {
            member_repo,
            event_repo,
            announcement_repo,
            payment_repo,
            integration_manager,
            auth_service,
            db_pool,
        }
    }
}