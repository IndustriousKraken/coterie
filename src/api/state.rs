use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::FromRef;
use axum::http::HeaderMap;
use sqlx::SqlitePool;
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    api::middleware::bot_challenge::BotChallengeVerifier,
    auth::{AuthService, CsrfService, PendingLoginService, TotpService},
    config::Settings,
    email::EmailSender,
    integrations::IntegrationManager,
    payments::{StripeClient, WebhookDispatcher},
    repository::{
        AnnouncementRepository, BasicTypeRepository, DonationCampaignRepository,
        EventRepository, EventSeriesRepository, ExpenseAccountRepository,
        ExpenseCategoryRepository, ExpenseRepository, MemberRepository,
        MembershipTypeRepository, PaymentRepository, ProcessedEventsRepository,
        SavedCardRepository, ScheduledPaymentRepository,
    },
    service::{
        announcement_admin_service::AnnouncementAdminService, audit_service::AuditService,
        basic_type_service::BasicTypeService, billing_service::BillingService,
        event_admin_service::EventAdminService,
        expense_account_service::ExpenseAccountService,
        expense_category_service::ExpenseCategoryService, expense_service::ExpenseService,
        member_service::MemberService, membership_type_service::MembershipTypeService,
        payment_admin_service::PaymentAdminService, payment_service::PaymentService,
        recurring_event_service::RecurringEventService, settings_service::SettingsService,
        ServiceContext,
    },
};

/// Extract client IP from request headers.
///
/// If `trust_forwarded` is false, X-Forwarded-For / X-Real-Ip are ignored
/// entirely (they can be spoofed by any client) and the fallback is used.
/// Set this based on whether the server sits behind a trusted reverse
/// proxy — see `ServerConfig::trust_forwarded_for`.
pub fn client_ip(headers: &HeaderMap, trust_forwarded: bool) -> IpAddr {
    if trust_forwarded {
        // Try X-Forwarded-For (first IP in the chain is the client)
        if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(first) = xff.split(',').next() {
                if let Ok(ip) = first.trim().parse::<IpAddr>() {
                    return ip;
                }
            }
        }
        // Try X-Real-Ip
        if let Some(xri) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            if let Ok(ip) = xri.trim().parse::<IpAddr>() {
                return ip;
            }
        }
    }
    // Fallback: localhost. We don't use ConnectInfo at the moment; when
    // `trust_forwarded` is false and the peer IP is unavailable, rate
    // limiting collapses to a single bucket. Safer than trusting a
    // client-supplied header.
    IpAddr::from([127, 0, 0, 1])
}

/// Simple in-memory rate limiter keyed by IP address.
#[derive(Clone)]
pub struct RateLimiter {
    /// Map of IP -> list of attempt timestamps within the window.
    attempts: Arc<Mutex<HashMap<IpAddr, Vec<Instant>>>>,
    /// Maximum attempts allowed within `window`.
    max_attempts: usize,
    /// Sliding window duration.
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_attempts: usize, window: Duration) -> Self {
        Self {
            attempts: Arc::new(Mutex::new(HashMap::new())),
            max_attempts,
            window,
        }
    }

    /// Returns `true` if the request is allowed, `false` if rate-limited.
    /// Automatically records the attempt when allowed.
    pub fn check_and_record(&self, ip: IpAddr) -> bool {
        // Recover from a poisoned mutex rather than propagating the
        // panic. A poisoned state means some prior call panicked while
        // holding the lock — the data may be slightly stale but the
        // rate limiter is best-effort anyway, and falling over here
        // would deny service to every login attempt.
        let mut map = match self.attempts.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!("RateLimiter mutex was poisoned; recovering");
                poisoned.into_inner()
            }
        };
        let now = Instant::now();
        let cutoff = now - self.window;

        let timestamps = map.entry(ip).or_default();
        timestamps.retain(|t| *t > cutoff);

        if timestamps.len() >= self.max_attempts {
            return false;
        }

        timestamps.push(now);
        true
    }

    /// Prune entries for IPs that have no recent attempts. Call periodically
    /// to prevent the map from growing unboundedly.
    pub fn cleanup(&self) {
        let mut map = match self.attempts.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let cutoff = Instant::now() - self.window;
        map.retain(|_, timestamps| {
            timestamps.retain(|t| *t > cutoff);
            !timestamps.is_empty()
        });
    }
}

#[derive(Clone)]
pub struct AppState {
    pub service_context: Arc<ServiceContext>,
    pub stripe_client: Option<Arc<StripeClient>>,
    /// Inbound webhook dispatcher. `Some` exactly when `stripe_client`
    /// is `Some` (both depend on Stripe being configured).
    pub webhook_dispatcher: Option<Arc<WebhookDispatcher>>,
    /// Billing operations (auto-renew lifecycle, dues extension, the
    /// scheduled-payment runner). Built once at startup; handlers
    /// borrow this Arc instead of reconstructing per-request — that
    /// pattern silently dropped state for any field with its own
    /// lifecycle, even though today's BillingService has no such
    /// field.
    pub billing_service: Arc<BillingService>,
    pub settings: Arc<Settings>,
    /// Rate limiter for login endpoints (5 attempts per 15 minutes per IP).
    pub login_limiter: RateLimiter,
    /// Rate limiter for money-moving endpoints (charge, donate, refund,
    /// auto-renew toggle). 10 attempts/min per IP — well above any
    /// legitimate workflow but tight enough to box in scripted abuse,
    /// double-submit accidents, and runaway clients. Per-IP rather
    /// than per-member because the source of an attack is the network,
    /// not the authenticated identity (which an attacker controlling
    /// a stolen session would also control).
    pub money_limiter: RateLimiter,
    /// Serializes first-admin setup to prevent concurrent requests from
    /// both passing the "no admin exists" check and creating two admins.
    pub setup_lock: Arc<AsyncMutex<()>>,
    /// Process-local cache for "has any admin been observed in the DB?".
    /// Set to true on the first positive lookup and never cleared. See
    /// `require_setup` for the lifecycle rationale.
    pub admin_exists_observed: Arc<AtomicBool>,
    /// Bot-challenge verifier. Gates `/public/signup` and
    /// `/public/donate` — see `api::middleware::bot_challenge`. When
    /// `bot_challenge.provider = "disabled"` (the default) this is the
    /// no-op `DisabledVerifier`, so existing dev flows keep working.
    pub bot_challenge_verifier: Arc<dyn BotChallengeVerifier>,
}

impl AppState {
    pub fn new(
        service_context: Arc<ServiceContext>,
        stripe_client: Option<Arc<StripeClient>>,
        webhook_dispatcher: Option<Arc<WebhookDispatcher>>,
        billing_service: Arc<BillingService>,
        settings: Arc<Settings>,
        bot_challenge_verifier: Arc<dyn BotChallengeVerifier>,
        money_limiter: MoneyLimiter,
    ) -> Self {
        Self {
            service_context,
            stripe_client,
            webhook_dispatcher,
            billing_service,
            settings,
            login_limiter: RateLimiter::new(5, Duration::from_secs(15 * 60)),
            money_limiter: money_limiter.0,
            setup_lock: Arc::new(AsyncMutex::new(())),
            admin_exists_observed: Arc::new(AtomicBool::new(false)),
            bot_challenge_verifier,
        }
    }
}

// FromRef<AppState> impls follow.
//
// Every constituent service, repository, and piece of infrastructure on
// AppState (and the ServiceContext reachable through it) has a FromRef
// impl below so handlers can write `State(svc): State<Arc<dyn X>>`
// instead of extracting the whole AppState. Adding a new field to
// AppState or ServiceContext SHOULD include a matching FromRef impl
// here — see the `routing-architecture` spec.

// --- Repositories ---

impl FromRef<AppState> for Arc<dyn MemberRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.member_repo.clone()
    }
}

impl FromRef<AppState> for Arc<dyn EventRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.event_repo.clone()
    }
}

impl FromRef<AppState> for Arc<dyn EventSeriesRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.event_series_repo.clone()
    }
}

impl FromRef<AppState> for Arc<dyn AnnouncementRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.announcement_repo.clone()
    }
}

impl FromRef<AppState> for Arc<dyn PaymentRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.payment_repo.clone()
    }
}

impl FromRef<AppState> for Arc<dyn SavedCardRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.saved_card_repo.clone()
    }
}

impl FromRef<AppState> for Arc<dyn ScheduledPaymentRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.scheduled_payment_repo.clone()
    }
}

impl FromRef<AppState> for Arc<dyn DonationCampaignRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.donation_campaign_repo.clone()
    }
}

impl FromRef<AppState> for Arc<dyn BasicTypeRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.basic_type_repo.clone()
    }
}

impl FromRef<AppState> for Arc<dyn MembershipTypeRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.membership_type_repo.clone()
    }
}

impl FromRef<AppState> for Arc<dyn ProcessedEventsRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.processed_events_repo.clone()
    }
}

impl FromRef<AppState> for Arc<dyn ExpenseRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.expense_repo.clone()
    }
}

impl FromRef<AppState> for Arc<dyn ExpenseCategoryRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.expense_category_repo.clone()
    }
}

impl FromRef<AppState> for Arc<dyn ExpenseAccountRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.expense_account_repo.clone()
    }
}

// --- Services ---

impl FromRef<AppState> for Arc<AuthService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.auth_service.clone()
    }
}

impl FromRef<AppState> for Arc<CsrfService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.csrf_service.clone()
    }
}

impl FromRef<AppState> for Arc<TotpService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.totp_service.clone()
    }
}

impl FromRef<AppState> for Arc<PendingLoginService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.pending_login_service.clone()
    }
}

impl FromRef<AppState> for Arc<SettingsService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.settings_service.clone()
    }
}

impl FromRef<AppState> for Arc<AuditService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.audit_service.clone()
    }
}

impl FromRef<AppState> for Arc<PaymentService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.payment_service.clone()
    }
}

impl FromRef<AppState> for Arc<RecurringEventService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.recurring_event_service.clone()
    }
}

impl FromRef<AppState> for Arc<MembershipTypeService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.membership_type_service.clone()
    }
}

// Two BasicTypeService instances share the same type — disambiguate via
// newtypes so handlers can extract whichever they need without ambiguity.

#[derive(Clone)]
pub struct EventBasicTypeService(pub Arc<BasicTypeService>);

#[derive(Clone)]
pub struct AnnouncementBasicTypeService(pub Arc<BasicTypeService>);

impl FromRef<AppState> for EventBasicTypeService {
    fn from_ref(state: &AppState) -> Self {
        EventBasicTypeService(state.service_context.event_type_service.clone())
    }
}

impl FromRef<AppState> for AnnouncementBasicTypeService {
    fn from_ref(state: &AppState) -> Self {
        AnnouncementBasicTypeService(state.service_context.announcement_type_service.clone())
    }
}

impl FromRef<AppState> for Arc<MemberService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.member_service.clone()
    }
}

impl FromRef<AppState> for Arc<EventAdminService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.event_admin_service.clone()
    }
}

impl FromRef<AppState> for Arc<AnnouncementAdminService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.announcement_admin_service.clone()
    }
}

impl FromRef<AppState> for Arc<PaymentAdminService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.payment_admin_service.clone()
    }
}

impl FromRef<AppState> for Arc<ExpenseService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.expense_service.clone()
    }
}

impl FromRef<AppState> for Arc<ExpenseCategoryService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.expense_category_service.clone()
    }
}

impl FromRef<AppState> for Arc<ExpenseAccountService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.expense_account_service.clone()
    }
}

impl FromRef<AppState> for Arc<dyn EmailSender> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.email_sender.clone()
    }
}

impl FromRef<AppState> for Arc<IntegrationManager> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.integration_manager.clone()
    }
}

// --- Infrastructure ---

impl FromRef<AppState> for Arc<BillingService> {
    fn from_ref(state: &AppState) -> Self {
        state.billing_service.clone()
    }
}

impl FromRef<AppState> for Option<Arc<StripeClient>> {
    fn from_ref(state: &AppState) -> Self {
        state.stripe_client.clone()
    }
}

impl FromRef<AppState> for Option<Arc<WebhookDispatcher>> {
    fn from_ref(state: &AppState) -> Self {
        state.webhook_dispatcher.clone()
    }
}

impl FromRef<AppState> for Arc<dyn BotChallengeVerifier> {
    fn from_ref(state: &AppState) -> Self {
        state.bot_challenge_verifier.clone()
    }
}

impl FromRef<AppState> for Arc<Settings> {
    fn from_ref(state: &AppState) -> Self {
        state.settings.clone()
    }
}

impl FromRef<AppState> for SqlitePool {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.db_pool.clone()
    }
}

// --- Rate limiters and locks ---
//
// RateLimiter appears twice on AppState (login_limiter, money_limiter), so
// a bare `FromRef<AppState> for RateLimiter` would be ambiguous. Each
// limiter gets a newtype wrapper.

#[derive(Clone)]
pub struct LoginLimiter(pub RateLimiter);

#[derive(Clone)]
pub struct MoneyLimiter(pub RateLimiter);

impl FromRef<AppState> for LoginLimiter {
    fn from_ref(state: &AppState) -> Self {
        LoginLimiter(state.login_limiter.clone())
    }
}

impl FromRef<AppState> for MoneyLimiter {
    fn from_ref(state: &AppState) -> Self {
        MoneyLimiter(state.money_limiter.clone())
    }
}

impl FromRef<AppState> for Arc<AsyncMutex<()>> {
    fn from_ref(state: &AppState) -> Self {
        state.setup_lock.clone()
    }
}

impl FromRef<AppState> for Arc<AtomicBool> {
    fn from_ref(state: &AppState) -> Self {
        state.admin_exists_observed.clone()
    }
}