//! Hot-swappable Stripe wiring.
//!
//! Stripe used to be built once at startup from `.env` and captured for
//! the process lifetime — changing a key meant editing `.env` and
//! restarting. This module holds the live `StripeClient` and inbound
//! `WebhookDispatcher` behind a swappable handle instead, so a portal
//! settings save can rebuild them and charges + webhook verification
//! pick up the new values with no restart.
//!
//! Reads are per-request: the `FromRef<AppState>` impls for
//! `Option<Arc<StripeClient>>` / `Option<Arc<WebhookDispatcher>>` call
//! [`StripeHandle::current`] each time, so every handler and the webhook
//! endpoint see the latest wiring automatically.

use std::sync::{Arc, RwLock};

use uuid::Uuid;

use crate::{
    config::nonblank,
    integrations::IntegrationManager,
    payments::{
        gateway::{RealStripeGateway, StripeGateway},
        StripeClient, WebhookDispatcher,
    },
    repository::{MemberRepository, PaymentRepository, ProcessedEventsRepository},
    service::{membership_type_service::MembershipTypeService, settings_service::SettingsService},
};

/// The current Stripe wiring. `client` and `webhook_dispatcher` are
/// `Some` exactly when Stripe is enabled and fully configured; both
/// `None` otherwise — disabled, a blank/whitespace secret, or an
/// undecryptable stored secret — so the webhook endpoint returns 503
/// rather than verify against a zero-length or broken HMAC key.
#[derive(Default, Clone)]
pub struct StripeRuntime {
    pub client: Option<Arc<StripeClient>>,
    pub webhook_dispatcher: Option<Arc<WebhookDispatcher>>,
}

/// Builds a `StripeGateway` from a secret key. Production wraps
/// `RealStripeGateway`; tests inject a `FakeStripeGateway` so the whole
/// rebuild path is exercisable without real Stripe calls.
pub type GatewayFactory = Arc<dyn Fn(String) -> Arc<dyn StripeGateway> + Send + Sync>;

/// Everything `rebuild` needs to construct a fresh client + dispatcher
/// from DB config. Absent for a preloaded (fixed) handle used in tests.
struct RebuildDeps {
    gateway_factory: GatewayFactory,
    settings_service: Arc<SettingsService>,
    payment_repo: Arc<dyn PaymentRepository>,
    member_repo: Arc<dyn MemberRepository>,
    processed_events_repo: Arc<dyn ProcessedEventsRepository>,
    membership_type_service: Arc<MembershipTypeService>,
    integration_manager: Arc<IntegrationManager>,
}

/// Swappable handle over the running Stripe wiring. Reads the current
/// DB config (decrypting the secrets) to (re)build a `StripeClient` +
/// `WebhookDispatcher`, then atomically swaps them in.
pub struct StripeHandle {
    runtime: RwLock<Arc<StripeRuntime>>,
    /// `None` for a preloaded/fixed handle (tests) — `rebuild` is then a
    /// no-op since there's no DB config to reload from.
    deps: Option<RebuildDeps>,
}

impl StripeHandle {
    /// Production constructor: gateways are real `RealStripeGateway`s.
    /// Starts with an empty (unconfigured) runtime — call
    /// [`StripeHandle::rebuild`] once at startup to load DB config.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        settings_service: Arc<SettingsService>,
        payment_repo: Arc<dyn PaymentRepository>,
        member_repo: Arc<dyn MemberRepository>,
        processed_events_repo: Arc<dyn ProcessedEventsRepository>,
        membership_type_service: Arc<MembershipTypeService>,
        integration_manager: Arc<IntegrationManager>,
    ) -> Self {
        Self::with_gateway_factory(
            Arc::new(|key| Arc::new(RealStripeGateway::new(key)) as Arc<dyn StripeGateway>),
            settings_service,
            payment_repo,
            member_repo,
            processed_events_repo,
            membership_type_service,
            integration_manager,
        )
    }

    /// Test seam: build a handle whose gateways come from an explicit
    /// factory (e.g. one that hands back a shared `FakeStripeGateway`).
    #[allow(clippy::too_many_arguments)]
    pub fn with_gateway_factory(
        gateway_factory: GatewayFactory,
        settings_service: Arc<SettingsService>,
        payment_repo: Arc<dyn PaymentRepository>,
        member_repo: Arc<dyn MemberRepository>,
        processed_events_repo: Arc<dyn ProcessedEventsRepository>,
        membership_type_service: Arc<MembershipTypeService>,
        integration_manager: Arc<IntegrationManager>,
    ) -> Self {
        Self {
            runtime: RwLock::new(Arc::new(StripeRuntime::default())),
            deps: Some(RebuildDeps {
                gateway_factory,
                settings_service,
                payment_repo,
                member_repo,
                processed_events_repo,
                membership_type_service,
                integration_manager,
            }),
        }
    }

    /// Test seam: a handle with a fixed runtime and no rebuild deps. The
    /// given client/dispatcher are what `current()` returns; `rebuild()`
    /// is a no-op. Lets router tests inject a fake-gateway client into
    /// the Stripe slot without wiring DB config.
    pub fn preloaded(
        client: Option<Arc<StripeClient>>,
        webhook_dispatcher: Option<Arc<WebhookDispatcher>>,
    ) -> Self {
        Self {
            runtime: RwLock::new(Arc::new(StripeRuntime {
                client,
                webhook_dispatcher,
            })),
            deps: None,
        }
    }

    /// Snapshot the current wiring. Cheap: a read-lock plus two Arc
    /// clones. Callers read this per request so a concurrent `rebuild`
    /// is picked up on the next call.
    pub fn current(&self) -> Arc<StripeRuntime> {
        self.runtime
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Rebuild the wiring from current DB config and swap it in. Called
    /// at startup and after every successful Stripe settings save. A
    /// preloaded (fixed) handle has no deps, so this is a no-op there.
    pub async fn rebuild(&self) {
        let Some(deps) = &self.deps else {
            return;
        };
        let next = Self::build_runtime(deps).await;
        let configured = next.client.is_some();
        *self.runtime.write().unwrap_or_else(|p| p.into_inner()) = Arc::new(next);
        if configured {
            tracing::info!("Stripe client rebuilt from DB config (enabled)");
        } else {
            tracing::info!("Stripe rebuilt from DB config: unconfigured/disabled (webhook → 503)");
        }
    }

    async fn build_runtime(deps: &RebuildDeps) -> StripeRuntime {
        // A decrypt failure (rotated session_secret) → treat as
        // unconfigured rather than build a client on a broken key.
        let cfg = match deps.settings_service.get_stripe_config().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Stripe config could not be decrypted ({}); treating Stripe as unconfigured",
                    e
                );
                return StripeRuntime::default();
            }
        };

        if !cfg.enabled {
            return StripeRuntime::default();
        }

        // Reuse `nonblank` so a blank/whitespace secret yields an
        // unconfigured client — no forgeable webhook against a
        // zero-length HMAC key. Both secrets are required.
        let (secret_key, webhook_secret) = match (
            nonblank(Some(cfg.secret_key)),
            nonblank(Some(cfg.webhook_secret)),
        ) {
            (Some(sk), Some(ws)) => (sk, ws),
            _ => return StripeRuntime::default(),
        };

        let gateway = (deps.gateway_factory)(secret_key);
        let client = Arc::new(StripeClient::with_gateway(
            gateway.clone(),
            deps.payment_repo.clone(),
            deps.member_repo.clone(),
        ));
        let dispatcher = Arc::new(WebhookDispatcher::new(
            gateway,
            webhook_secret,
            deps.payment_repo.clone(),
            deps.member_repo.clone(),
            deps.processed_events_repo.clone(),
            deps.membership_type_service.clone(),
            deps.integration_manager.clone(),
        ));

        StripeRuntime {
            client: Some(client),
            webhook_dispatcher: Some(dispatcher),
        }
    }
}

/// The system actor id used for startup writes (the one-time `.env`
/// seed) that aren't attributable to a logged-in admin.
pub const SYSTEM_ACTOR: Uuid = Uuid::nil();
