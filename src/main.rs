mod api;
mod auth;
mod config;
mod domain;
mod error;
mod integrations;
mod payments;
mod repository;
mod service;
mod web;

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
    let event_repo = Arc::new(repository::SqliteEventRepository::new(db_pool.clone()));
    let announcement_repo = Arc::new(repository::SqliteAnnouncementRepository::new(db_pool.clone()));
    let payment_repo = Arc::new(repository::SqlitePaymentRepository::new(db_pool.clone()));

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
        payment_repo.clone(),
        integration_manager,
        auth_service,
        db_pool.clone(),
    ));

    // Initialize Stripe client if configured
    let stripe_client = if settings.stripe.enabled {
        if let (Some(api_key), Some(webhook_secret)) = 
            (settings.stripe.secret_key.clone(), settings.stripe.webhook_secret.clone()) {
            tracing::info!("Stripe payment processing enabled");
            Some(Arc::new(payments::StripeClient::new(
                api_key,
                webhook_secret,
                payment_repo,
            )))
        } else {
            tracing::warn!("Stripe enabled but missing configuration");
            None
        }
    } else {
        tracing::info!("Stripe payment processing disabled");
        None
    };

    // Create API app
    let api_app = api::create_app(service_context.clone(), stripe_client.clone(), Arc::new(settings.clone()));
    
    // Create web app state separately
    let web_app_state = api::state::AppState::new(
        service_context,
        stripe_client,
        Arc::new(settings.clone()),
    );
    let web_app = web::create_web_routes(web_app_state);
    
    // Combine API and web routes
    let app = api_app.merge(web_app);

    let listener = tokio::net::TcpListener::bind(
        format!("{}:{}", settings.server.host, settings.server.port)
    ).await?;

    tracing::info!("Server listening on http://{}:{}", settings.server.host, settings.server.port);

    axum::serve(listener, app).await?;

    Ok(())
}