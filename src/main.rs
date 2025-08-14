mod api;
mod auth;
mod config;
mod domain;
mod error;
mod integrations;
mod repository;
mod service;

use std::sync::Arc;
use sqlx::sqlite::SqlitePoolOptions;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    config::Settings,
    integrations::{IntegrationManager, discord::DiscordIntegration, unifi::UnifiIntegration},
    service::ServiceContext,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "coterie=debug,tower_http=debug,axum=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let settings = Settings::new().unwrap_or_else(|e| {
        tracing::warn!("Failed to load config: {}. Using defaults.", e);
        Settings::default()
    });

    tracing::info!("Starting Coterie server on {}:{}", settings.server.host, settings.server.port);

    // Initialize database
    let db_pool = SqlitePoolOptions::new()
        .max_connections(settings.database.max_connections)
        .connect(&settings.database.url)
        .await?;

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&db_pool)
        .await?;

    // Initialize auth service
    let auth_service = Arc::new(auth::AuthService::new(
        db_pool.clone(),
        settings.auth.session_secret.clone(),
    ));

    // Initialize repositories
    let member_repo = Arc::new(repository::SqliteMemberRepository::new(db_pool.clone()));
    let event_repo = Arc::new(StubEventRepo {});
    let announcement_repo = Arc::new(StubAnnouncementRepo {});
    let payment_repo = Arc::new(StubPaymentRepo {});

    // Initialize integration manager
    let integration_manager = Arc::new(IntegrationManager::new());

    // Register integrations
    if let Some(discord) = DiscordIntegration::new(settings.integrations.discord.clone()) {
        integration_manager.register(Arc::new(discord)).await;
    }

    if let Some(unifi) = UnifiIntegration::new(settings.integrations.unifi.clone()) {
        integration_manager.register(Arc::new(unifi)).await;
    }

    // Check integration health
    let health_results = integration_manager.health_check_all().await;
    for (name, result) in health_results {
        match result {
            Ok(_) => tracing::info!("Integration {} is healthy", name),
            Err(e) => tracing::warn!("Integration {} health check failed: {:?}", name, e),
        }
    }

    // Create service context
    let service_context = Arc::new(ServiceContext::new(
        member_repo,
        event_repo,
        announcement_repo,
        payment_repo,
        integration_manager,
        auth_service,
        db_pool.clone(),
    ));

    // Create and run app
    let app = api::create_app(service_context);

    let listener = tokio::net::TcpListener::bind(
        format!("{}:{}", settings.server.host, settings.server.port)
    ).await?;

    tracing::info!("Server listening on http://{}:{}", settings.server.host, settings.server.port);

    axum::serve(listener, app).await?;

    Ok(())
}

// Temporary stub implementations - these would be replaced with actual SQLite implementations
struct StubMemberRepo;
struct StubEventRepo;
struct StubAnnouncementRepo;
struct StubPaymentRepo;

use async_trait::async_trait;
use uuid::Uuid;
use crate::domain::*;
use crate::error::Result;
use crate::repository::*;

#[async_trait]
impl MemberRepository for StubMemberRepo {
    async fn create(&self, _member: CreateMemberRequest) -> Result<Member> {
        unimplemented!("SQLite implementation needed")
    }
    async fn find_by_id(&self, _id: Uuid) -> Result<Option<Member>> {
        Ok(None)
    }
    async fn find_by_email(&self, _email: &str) -> Result<Option<Member>> {
        Ok(None)
    }
    async fn find_by_username(&self, _username: &str) -> Result<Option<Member>> {
        Ok(None)
    }
    async fn list(&self, _limit: i64, _offset: i64) -> Result<Vec<Member>> {
        Ok(vec![])
    }
    async fn list_active(&self) -> Result<Vec<Member>> {
        Ok(vec![])
    }
    async fn list_expired(&self) -> Result<Vec<Member>> {
        Ok(vec![])
    }
    async fn update(&self, _id: Uuid, _update: UpdateMemberRequest) -> Result<Member> {
        unimplemented!("SQLite implementation needed")
    }
    async fn delete(&self, _id: Uuid) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl EventRepository for StubEventRepo {
    async fn create(&self, _event: Event) -> Result<Event> {
        unimplemented!("SQLite implementation needed")
    }
    async fn find_by_id(&self, _id: Uuid) -> Result<Option<Event>> {
        Ok(None)
    }
    async fn list_upcoming(&self, _limit: i64) -> Result<Vec<Event>> {
        Ok(vec![])
    }
    async fn list_public(&self) -> Result<Vec<Event>> {
        Ok(vec![])
    }
    async fn update(&self, _id: Uuid, _event: Event) -> Result<Event> {
        unimplemented!("SQLite implementation needed")
    }
    async fn delete(&self, _id: Uuid) -> Result<()> {
        Ok(())
    }
    async fn register_attendance(&self, _event_id: Uuid, _member_id: Uuid) -> Result<()> {
        Ok(())
    }
    async fn cancel_attendance(&self, _event_id: Uuid, _member_id: Uuid) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl AnnouncementRepository for StubAnnouncementRepo {
    async fn create(&self, _announcement: Announcement) -> Result<Announcement> {
        unimplemented!("SQLite implementation needed")
    }
    async fn find_by_id(&self, _id: Uuid) -> Result<Option<Announcement>> {
        Ok(None)
    }
    async fn list_recent(&self, _limit: i64) -> Result<Vec<Announcement>> {
        Ok(vec![])
    }
    async fn list_public(&self) -> Result<Vec<Announcement>> {
        Ok(vec![])
    }
    async fn list_featured(&self) -> Result<Vec<Announcement>> {
        Ok(vec![])
    }
    async fn update(&self, _id: Uuid, _announcement: Announcement) -> Result<Announcement> {
        unimplemented!("SQLite implementation needed")
    }
    async fn delete(&self, _id: Uuid) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl PaymentRepository for StubPaymentRepo {
    async fn create(&self, _payment: Payment) -> Result<Payment> {
        unimplemented!("SQLite implementation needed")
    }
    async fn find_by_id(&self, _id: Uuid) -> Result<Option<Payment>> {
        Ok(None)
    }
    async fn find_by_member(&self, _member_id: Uuid) -> Result<Vec<Payment>> {
        Ok(vec![])
    }
    async fn find_by_stripe_id(&self, _stripe_id: &str) -> Result<Option<Payment>> {
        Ok(None)
    }
    async fn update_status(&self, _id: Uuid, _status: PaymentStatus) -> Result<Payment> {
        unimplemented!("SQLite implementation needed")
    }
}