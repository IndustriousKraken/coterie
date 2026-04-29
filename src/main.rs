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
use sqlx::{Executor, sqlite::SqlitePoolOptions};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    config::Settings,
    integrations::{
        IntegrationManager,
        admin_alert_email::AdminAlertEmailIntegration,
        discord::DiscordIntegration,
        unifi::UnifiIntegration,
    },
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

    // Foreign keys are off by default in SQLite — every new
    // connection from the pool needs PRAGMA foreign_keys = ON for
    // the FK constraints in our migrations to actually fire. Without
    // this they're decorative; orphan payment / saved_card / etc.
    // rows can be inserted with member_ids that don't exist.
    let db_pool = SqlitePoolOptions::new()
        .max_connections(settings.database.max_connections)
        .after_connect(|conn, _meta| Box::pin(async move {
            conn.execute("PRAGMA foreign_keys = ON").await?;
            Ok(())
        }))
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

    // CSRF tokens are stateless HMAC; the service derives its key from
    // session_secret so rotating that secret invalidates outstanding
    // tokens (users get a 403 on next submit and retry).
    let csrf_service = Arc::new(auth::CsrfService::new(&settings.auth.session_secret));

    // TOTP / 2FA. Issuer is the org name shown in authenticator apps;
    // we look it up once at startup, fall back to "Coterie" if unset.
    // Live org-name changes don't propagate without restart, but
    // existing enrollments aren't affected (issuer is metadata in the
    // enrolled otpauth URL, not part of the verification math).
    let totp_issuer = settings_service
        .get_setting("org.name").await
        .ok()
        .map(|s| s.value)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Coterie".to_string());
    let totp_service = Arc::new(auth::TotpService::new(
        db_pool.clone(),
        crypto.clone(),
        totp_issuer,
    ));
    let pending_login_service = Arc::new(auth::PendingLoginService::new(db_pool.clone()));

    // Initialize repositories
    let member_repo = Arc::new(repository::SqliteMemberRepository::new(db_pool.clone()));
    let event_repo = Arc::new(repository::SqliteEventRepository::new(db_pool.clone()));
    let announcement_repo = Arc::new(repository::SqliteAnnouncementRepository::new(db_pool.clone()));
    let payment_repo = Arc::new(repository::SqlitePaymentRepository::new(db_pool.clone()));

    // Initialize integration manager
    let integration_manager = Arc::new(IntegrationManager::new());

    // Discord: always register the integration object — it reads its
    // own config from the DB on each event and skips silently when
    // disabled. Admin can flip discord.enabled at runtime via the
    // settings page without a restart.
    //
    // Keep a separate handle so the daily reconcile task can call
    // its concrete `reconcile_all` method (the Integration trait
    // intentionally doesn't expose Discord-specific operations).
    let discord_integration = Arc::new(DiscordIntegration::new(
        settings_service.clone(),
        settings.server.base_url.clone(),
    ));
    integration_manager
        .register(discord_integration.clone())
        .await;

    // Email backup for AdminAlert events: ensures critical
    // notifications still reach operators when Discord is down or
    // unconfigured. Sends to org.contact_email.
    integration_manager
        .register(Arc::new(AdminAlertEmailIntegration::new(
            settings_service.clone(),
            email_sender.clone(),
        )))
        .await;

    // Unifi: still env-var-driven for now (D5+ scope). Skip if config
    // is absent.
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
        csrf_service,
        totp_service,
        pending_login_service,
        db_pool.clone(),
    ));

    // Spawn background cleanup task (runs hourly) for expired sessions
    // and for pruning old audit-log entries based on the operator-set
    // retention window.
    {
        let auth_service = service_context.auth_service.clone();
        let audit_service = service_context.audit_service.clone();
        let settings_service = service_context.settings_service.clone();
        let cleanup_pool = db_pool.clone();
        tokio::spawn(async move {
            let cleanup_interval = tokio::time::Duration::from_secs(60 * 60); // 1 hour
            loop {
                tokio::time::sleep(cleanup_interval).await;

                // Expired sessions
                match auth_service.cleanup_expired_sessions().await {
                    Ok(count) if count > 0 => {
                        tracing::info!("Cleaned up {} expired sessions", count);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to cleanup expired sessions: {:?}", e);
                    }
                    _ => {}
                }

                // Audit-log retention (default 365 days, clamped in
                // `prune_older_than` to sane bounds).
                let retention_days = settings_service
                    .get_number("audit.retention_days")
                    .await
                    .unwrap_or(365);
                match audit_service.prune_older_than(retention_days).await {
                    Ok(count) if count > 0 => {
                        tracing::info!("Pruned {} audit-log entries older than {} days", count, retention_days);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to prune audit log: {:?}", e);
                    }
                    _ => {}
                }

                // Stripe webhook idempotency table. Stripe retries for
                // ~3 days max; anything older than 30 days has zero
                // chance of a legitimate replay. Without this prune
                // the table grows unbounded over the lifetime of the
                // deployment.
                match sqlx::query(
                    "DELETE FROM processed_stripe_events \
                     WHERE processed_at < datetime('now', '-30 days')",
                )
                .execute(&cleanup_pool)
                .await
                {
                    Ok(res) if res.rows_affected() > 0 => {
                        tracing::info!(
                            "Pruned {} processed_stripe_events older than 30 days",
                            res.rows_affected(),
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Failed to prune processed_stripe_events: {:?}", e);
                    }
                    _ => {}
                }
            }
        });
    }

    // Spawn daily Discord role reconcile. Catches drift from any
    // events that didn't deliver during a Discord outage. Cheap
    // enough at the volumes we expect (<1k members) that running
    // every 24h is fine. The integration itself no-ops when Discord
    // is disabled or unconfigured.
    {
        let discord = discord_integration.clone();
        let members = service_context.member_repo.clone();
        tokio::spawn(async move {
            let interval = tokio::time::Duration::from_secs(24 * 60 * 60);
            // Initial delay so we don't hammer Discord during a
            // restart loop and so the first reconcile runs in the
            // background after the server has settled.
            tokio::time::sleep(tokio::time::Duration::from_secs(5 * 60)).await;
            loop {
                let summary = discord.reconcile_all(members.clone()).await;
                tracing::info!(
                    "Discord daily reconcile: processed={}, skipped_invalid_id={}, skipped_pending={}",
                    summary.processed, summary.skipped_invalid_id, summary.skipped_pending,
                );
                tokio::time::sleep(interval).await;
            }
        });
    }

    // Spawn daily recurring-event horizon extension. Each active
    // series gets its `materialized_through` rolled forward to
    // (today + 12 months). One-time runs at startup catch any drift
    // from prolonged downtime; the daily cadence keeps the calendar
    // perpetually showing a year of meetings without operator action.
    {
        let recurring = service_context.recurring_event_service.clone();
        tokio::spawn(async move {
            let interval = tokio::time::Duration::from_secs(24 * 60 * 60);
            // Run once shortly after boot so a fresh deploy with
            // existing series catches up before the first daily tick.
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            loop {
                match recurring.extend_horizon_for_active_series().await {
                    Ok(0) => {
                        tracing::debug!("Recurring-event horizon extend: nothing to do");
                    }
                    Ok(n) => {
                        tracing::info!("Recurring-event horizon extend: added {} occurrences", n);
                    }
                    Err(e) => {
                        tracing::error!("Recurring-event horizon extend failed: {}", e);
                    }
                }
                tokio::time::sleep(interval).await;
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
                service_context.member_repo.clone(),
                service_context.membership_type_service.clone(),
                service_context.integration_manager.clone(),
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

    // And for the money-endpoint limiter. Shorter window means we
    // sweep more often to keep the per-IP map small.
    {
        let limiter = web_app_state.money_limiter.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
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