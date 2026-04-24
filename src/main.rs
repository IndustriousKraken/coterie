mod api;
mod auth;
mod config;
mod domain;
mod email;
mod error;
mod integrations;
mod jobs;
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
    // Load .env file if present (before anything else)
    dotenvy::dotenv().ok();

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "coterie=debug,tower_http=debug,axum=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration — crash on missing/invalid config rather than silently using defaults
    let settings = Settings::new().expect(
        "Failed to load configuration. \
         Ensure .env exists with all required fields (see .env.example)."
    );

    tracing::info!("Starting Coterie server on {}:{}", settings.server.host, settings.server.port);

    // Initialize database (resolves path relative to data_dir if needed)
    let database_url = settings.database_url();
    tracing::info!("Using database: {}", database_url);

    // Ensure data directory exists
    let data_dir = std::path::Path::new(&settings.server.data_dir);
    if !data_dir.exists() {
        std::fs::create_dir_all(data_dir)?;
    }

    let db_pool = SqlitePoolOptions::new()
        .max_connections(settings.database.max_connections)
        .connect(&database_url)
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

    // Encryption helper for secrets-at-rest (e.g. SMTP password in
    // settings). Key is derived from session_secret — if the operator
    // rotates that, encrypted settings become unreadable and must be
    // re-entered.
    let crypto = Arc::new(auth::SecretCrypto::new(&settings.auth.session_secret));

    // Settings service needs to exist before both the ServiceContext
    // (which holds it) and the email sender (which reads live config
    // from it on every send).
    let settings_service = Arc::new(service::settings_service::SettingsService::new(
        db_pool.clone(),
        crypto.clone(),
    ));

    // Email sender reads config from the DB at send time so admins can
    // change SMTP settings from the UI without a restart.
    let email_sender: Arc<dyn email::EmailSender> =
        Arc::new(email::DynamicSender::new(settings_service.clone()));

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
        email_sender,
        settings_service,
        db_pool.clone(),
    ));

    // Spawn background cleanup task for expired sessions, CSRF tokens, and rate-limit state
    {
        let auth_service = service_context.auth_service.clone();
        let csrf_service = service_context.csrf_service.clone();
        tokio::spawn(async move {
            let cleanup_interval = tokio::time::Duration::from_secs(60 * 60); // 1 hour
            loop {
                tokio::time::sleep(cleanup_interval).await;

                // Cleanup expired sessions
                match auth_service.cleanup_expired_sessions().await {
                    Ok(count) if count > 0 => {
                        tracing::info!("Cleaned up {} expired sessions", count);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to cleanup expired sessions: {:?}", e);
                    }
                    _ => {}
                }

                // Cleanup orphaned CSRF tokens
                match csrf_service.cleanup_orphaned().await {
                    Ok(count) if count > 0 => {
                        tracing::info!("Cleaned up {} orphaned CSRF tokens", count);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to cleanup orphaned CSRF tokens: {:?}", e);
                    }
                    _ => {}
                }
            }
        });
    }

    // Initialize Stripe client if configured
    let stripe_client = if settings.stripe.enabled {
        if let (Some(api_key), Some(webhook_secret)) =
            (settings.stripe.secret_key.clone(), settings.stripe.webhook_secret.clone()) {
            tracing::info!("Stripe payment processing enabled");
            Some(Arc::new(payments::StripeClient::new(
                api_key,
                webhook_secret,
                payment_repo,
                service_context.membership_type_service.clone(),
                db_pool.clone(),
            )))
        } else {
            tracing::warn!("Stripe enabled but missing configuration");
            None
        }
    } else {
        tracing::info!("Stripe payment processing disabled");
        None
    };

    // Spawn billing runner (runs every hour)
    {
        let billing_service = Arc::new(service_context.billing_service(
            stripe_client.clone(),
            settings.server.base_url.clone(),
        ));
        let runner = jobs::BillingRunner::new(billing_service, 60 * 60);
        runner.spawn();
        tracing::info!("Billing runner spawned");
    }

    // Create API app
    let api_app = api::create_app(service_context.clone(), stripe_client.clone(), Arc::new(settings.clone()));

    // Create web app state separately
    let web_app_state = api::state::AppState::new(
        service_context,
        stripe_client,
        Arc::new(settings.clone()),
    );

    // Spawn periodic cleanup for the login rate limiter
    {
        let limiter = web_app_state.login_limiter.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(15 * 60)).await;
                limiter.cleanup();
            }
        });
    }

    let web_app = web::create_web_routes(web_app_state.clone());

    // Combine API and web routes, apply setup check middleware
    let app = api_app
        .merge(web_app)
        .layer(axum::middleware::from_fn_with_state(
            web_app_state,
            api::middleware::setup::require_setup,
        ));

    let listener = tokio::net::TcpListener::bind(
        format!("{}:{}", settings.server.host, settings.server.port)
    ).await?;

    tracing::info!("Server listening on http://{}:{}", settings.server.host, settings.server.port);

    axum::serve(listener, app).await?;

    Ok(())
}