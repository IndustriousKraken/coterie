use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
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
    payments::{StripeClient, StripeHandle, WebhookDispatcher},
    repository::{
        AnnouncementRepository, BasicTypeRepository, DonationCampaignRepository, EventRepository,
        EventSeriesRepository, ExpenseAccountRepository, ExpenseCategoryRepository,
        ExpenseRepository, MemberRepository, MembershipTypeRepository, PaymentRepository,
        ProcessedEventsRepository, SavedCardRepository, ScheduledPaymentRepository,
        SeriesEnrollmentRepository, SubmissionRepository,
    },
    service::{
        announcement_admin_service::AnnouncementAdminService, audit_service::AuditService,
        basic_type_service::BasicTypeService, billing_service::BillingService,
        event_admin_service::EventAdminService,
        event_registration_service::EventRegistrationService,
        expense_account_service::ExpenseAccountService,
        expense_category_service::ExpenseCategoryService, expense_service::ExpenseService,
        member_service::MemberService, membership_type_service::MembershipTypeService,
        payment_admin_service::PaymentAdminService, payment_service::PaymentService,
        recurring_event_service::RecurringEventService,
        series_enrollment_service::SeriesEnrollmentService, settings_service::SettingsService,
        submission_service::SubmissionService, ServiceContext,
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
        // Take the RIGHT-most X-Forwarded-For entry: the hop appended by the
        // single trusted proxy (Caddy). A standard proxy APPENDS the peer it
        // received the connection from, so any left-of-that entries are
        // client-supplied and spoofable — reading `.next()` (left-most) would
        // let an attacker rotate the rate-limit key per request.
        // ponytail: single trusted proxy => right-most; add a hop-count knob
        // only if a multi-proxy deployment ever needs it.
        if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(last) = xff.split(',').next_back() {
                if let Ok(ip) = last.trim().parse::<IpAddr>() {
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

/// Lock a limiter map, recovering from a poisoned mutex rather than
/// propagating the panic. A poisoned state means some prior call
/// panicked while holding the lock — the data may be slightly stale but
/// rate limiting is best-effort anyway, and falling over here would deny
/// service to every login attempt.
fn lock_recovering<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| {
        tracing::warn!("rate limiter mutex was poisoned; recovering");
        poisoned.into_inner()
    })
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
    ///
    /// `endpoint` names the endpoint class for the `auth.rate_limited`
    /// event emitted on rejection. It is a parameter rather than a
    /// per-call-site log line so a new limiter call site cannot forget
    /// it — 174 rejections went unlogged in production before this.
    /// Rejections are log-only: their volume is attacker-controlled, so
    /// an `audit_logs` row per trip would be a write-amplification lever.
    pub fn check_and_record(&self, ip: IpAddr, endpoint: &str) -> bool {
        let mut map = lock_recovering(&self.attempts);
        let now = Instant::now();
        let cutoff = now - self.window;

        let timestamps = map.entry(ip).or_default();
        timestamps.retain(|t| *t > cutoff);

        if timestamps.len() >= self.max_attempts {
            crate::util::auth_log::denied(
                "auth.rate_limited",
                "rate_limited",
                None,
                Some(ip),
                None,
                Some(endpoint),
            );
            return false;
        }

        timestamps.push(now);
        true
    }

    /// Prune entries for IPs that have no recent attempts. Call periodically
    /// to prevent the map from growing unboundedly.
    pub fn cleanup(&self) {
        let mut map = lock_recovering(&self.attempts);
        let cutoff = Instant::now() - self.window;
        map.retain(|_, timestamps| {
            timestamps.retain(|t| *t > cutoff);
            !timestamps.is_empty()
        });
    }
}

/// Failure budget for the credential endpoints (`/auth/login`, `/login`,
/// `/auth/login/totp`, `/login/totp`).
///
/// Two things separate this from [`RateLimiter`]:
///
/// 1. **Only failures count.** [`Self::check`] is read-only; the caller
///    calls [`Self::record_failure`] after verification fails, so a
///    successful authentication consumes nothing. Counting successes
///    rationed the outcome the limiter exists to protect: on 2026-08-13
///    an administrator was locked out of production by five consecutive
///    *successful* logins — one of them a correct second factor, two of
///    them a single double-submitted form. A success costing nothing is
///    also why no de-duplication of repeated submissions exists here.
/// 2. **The tight budget keys on the account, not the address.** Brute
///    force targets an account, so that is where a tight budget belongs.
///    Ten members at an event venue present one NAT address; under a
///    per-address budget a few mistyped passwords among them denied
///    login to everyone in the room, including members who had attempted
///    nothing.
///
/// The address budget survives as a deliberately hard-to-reach
/// sustained-abuse path, measured by the **breadth** of accounts a
/// source fails against rather than by raw failure count. A room of
/// members produces failures concentrated on a few accounts by people
/// who roughly know their own passwords; credential stuffing spreads
/// thin across many accounts, a large share of them names that match no
/// member. A raw count cannot tell those apart.
#[derive(Clone)]
pub struct CredentialLimiter {
    /// Normalized identifier -> failure timestamps within the window.
    account_failures: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    /// Address -> (normalized identifier -> its most recent failure).
    /// The budget is the SIZE of the inner map, i.e. breadth. Failures
    /// against identifiers matching no member count here too: a source
    /// guessing at accounts that do not exist is not a member mistyping
    /// their own password.
    address_breadth: Arc<Mutex<HashMap<IpAddr, HashMap<String, Instant>>>>,
    /// Failures allowed per account within `window`.
    max_failures: usize,
    /// Distinct accounts one address may fail against within `window`.
    max_accounts: usize,
    window: Duration,
}

impl CredentialLimiter {
    pub fn new(max_failures: usize, max_accounts: usize, window: Duration) -> Self {
        Self {
            account_failures: Arc::new(Mutex::new(HashMap::new())),
            address_breadth: Arc::new(Mutex::new(HashMap::new())),
            max_failures,
            max_accounts,
            window,
        }
    }

    /// The budget key for a submitted identifier.
    ///
    /// Keyed **as submitted** — normalized, but never required to
    /// resolve to a member, so that spending budget cannot reveal which
    /// accounts exist. Trimmed and lower-cased because login treats
    /// those as the same account and an attacker must not get a fresh
    /// budget per capitalization. Bounded in length because the key is
    /// attacker-supplied and the map holds it for the window.
    pub fn account_key(identifier: &str) -> String {
        identifier.trim().to_lowercase().chars().take(254).collect()
    }

    /// Read-only: is this attempt within both budgets? Records nothing.
    ///
    /// Call this BEFORE verifying credentials — an attempt that is
    /// already over the limit must do no password hashing, which is the
    /// difference between a rate limiter and a hashing-DoS amplifier.
    ///
    /// `endpoint` names the endpoint class for the `auth.rate_limited`
    /// event emitted on rejection. It is a parameter rather than a
    /// per-call-site log line so a new limiter call site cannot forget
    /// it — 174 rejections went unlogged in production before this.
    /// Rejections are log-only: their volume is attacker-controlled, so
    /// an `audit_logs` row per trip would be a write-amplification lever.
    pub fn check(&self, ip: IpAddr, identifier: &str, endpoint: &str) -> bool {
        let key = Self::account_key(identifier);
        let cutoff = Instant::now() - self.window;

        let account_failures = lock_recovering(&self.account_failures)
            .get(&key)
            .map_or(0, |ts| ts.iter().filter(|t| **t > cutoff).count());
        let address_breadth = lock_recovering(&self.address_breadth)
            .get(&ip)
            .map_or(0, |ids| ids.values().filter(|t| **t > cutoff).count());

        // Either budget rejects, and the rejection is the same one: the
        // caller must not learn which budget it hit, because "the
        // address budget" and "this account's budget" would together
        // answer whether the account exists.
        if account_failures >= self.max_failures || address_breadth >= self.max_accounts {
            crate::util::auth_log::denied(
                "auth.rate_limited",
                "rate_limited",
                None,
                Some(ip),
                None,
                Some(endpoint),
            );
            return false;
        }
        true
    }

    /// Spend budget. Call this ONLY after credential verification has
    /// failed — a successful authentication spends nothing.
    pub fn record_failure(&self, ip: IpAddr, identifier: &str) {
        let key = Self::account_key(identifier);
        let now = Instant::now();
        let cutoff = now - self.window;

        {
            let mut map = lock_recovering(&self.account_failures);
            let timestamps = map.entry(key.clone()).or_default();
            timestamps.retain(|t| *t > cutoff);
            timestamps.push(now);
        }

        let mut map = lock_recovering(&self.address_breadth);
        let identifiers = map.entry(ip).or_default();
        identifiers.retain(|_, t| *t > cutoff);
        // Overwrite rather than append: five failures against one
        // account are one account's worth of breadth, so a member
        // locking themselves out advances this by exactly 1 and stays a
        // private matter between them and their own account.
        identifiers.insert(key, now);
    }

    /// Prune entries with no recent failures. Call periodically: the
    /// per-address maps are self-bounding (once breadth is reached
    /// nothing more is recorded from that address), but the set of
    /// addresses and accounts seen over a long uptime is not.
    pub fn cleanup(&self) {
        let cutoff = Instant::now() - self.window;
        lock_recovering(&self.account_failures).retain(|_, timestamps| {
            timestamps.retain(|t| *t > cutoff);
            !timestamps.is_empty()
        });
        lock_recovering(&self.address_breadth).retain(|_, identifiers| {
            identifiers.retain(|_, t| *t > cutoff);
            !identifiers.is_empty()
        });
    }
}

#[derive(Clone)]
pub struct AppState {
    pub service_context: Arc<ServiceContext>,
    /// Hot-swappable Stripe wiring (client + inbound webhook
    /// dispatcher). Held behind a handle rather than captured once at
    /// startup so a portal settings save takes effect without a restart.
    /// Handlers read the current value per request via the
    /// `Option<Arc<StripeClient>>` / `Option<Arc<WebhookDispatcher>>`
    /// FromRef impls below.
    pub stripe: Arc<StripeHandle>,
    /// Billing operations (auto-renew lifecycle, dues extension, the
    /// scheduled-payment runner). Built once at startup; handlers
    /// borrow this Arc instead of reconstructing per-request — that
    /// pattern silently dropped state for any field with its own
    /// lifecycle, even though today's BillingService has no such
    /// field.
    pub billing_service: Arc<BillingService>,
    pub settings: Arc<Settings>,
    /// Failure budget for the login endpoints: 5 failed attempts per 15
    /// minutes per **account**, plus a per-address breadth allowance.
    /// Successes cost nothing — see [`CredentialLimiter`].
    pub login_limiter: CredentialLimiter,
    /// Rate limiter for password recovery (`POST /forgot-password`).
    /// Same 5-per-15-minutes shape as `login_limiter` but a SEPARATE
    /// bucket: a member who has just failed five logins is exactly the
    /// member most likely to need a reset, and sharing the credential
    /// budget locked them out of the one door built for them (incident
    /// 2026-07-29). Recovery is still limited because it sends email.
    pub recovery_limiter: RateLimiter,
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
    /// Cache of the latest known stable release, refreshed by the daily
    /// update-check task and read by the portal "update available"
    /// banner. The backing store is the singleton in
    /// `service::update_check`, so the static `BaseContext::for_member`
    /// render helper reads the same value this task writes without the
    /// cache having to be threaded through every portal handler. See
    /// a40 / D2.
    pub latest_release: LatestReleaseCache,
}

impl AppState {
    pub fn new(
        service_context: Arc<ServiceContext>,
        stripe: Arc<StripeHandle>,
        billing_service: Arc<BillingService>,
        settings: Arc<Settings>,
        bot_challenge_verifier: Arc<dyn BotChallengeVerifier>,
        money_limiter: MoneyLimiter,
    ) -> Self {
        Self {
            service_context,
            stripe,
            billing_service,
            settings,
            // 5 failures per account, and 20 distinct accounts per
            // address, both per 15 minutes.
            //
            // 20 is chosen against the venue case: ten members behind
            // one NAT address, several mistyping, produce breadth equal
            // to the number of *members* fumbling — one member burning
            // all five of their own failures still advances this by 1.
            // Twice the venue leaves the room room to spare while
            // stuffing, which sprays one or two tries across dozens of
            // accounts, crosses it almost immediately.
            // ponytail: one global number; make it configurable only if
            // a deployment reports a legitimate address exceeding it.
            login_limiter: CredentialLimiter::new(5, 20, Duration::from_secs(15 * 60)),
            recovery_limiter: RateLimiter::new(5, Duration::from_secs(15 * 60)),
            money_limiter: money_limiter.0,
            setup_lock: Arc::new(AsyncMutex::new(())),
            admin_exists_observed: Arc::new(AtomicBool::new(false)),
            bot_challenge_verifier,
            // Same Arc the background task and the render path use.
            latest_release: LatestReleaseCache(crate::service::update_check::cache()),
        }
    }
}

/// Handle to the process-wide latest-stable-release cache. Newtype so
/// it has a single unambiguous `FromRef<AppState>` impl. The inner
/// `Arc` is the singleton from `service::update_check`.
#[derive(Clone)]
pub struct LatestReleaseCache(pub Arc<RwLock<Option<crate::service::update_check::LatestRelease>>>);

impl FromRef<AppState> for LatestReleaseCache {
    fn from_ref(state: &AppState) -> Self {
        state.latest_release.clone()
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

impl FromRef<AppState> for Arc<dyn SeriesEnrollmentRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.series_enrollment_repo.clone()
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

impl FromRef<AppState> for Arc<dyn SubmissionRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.submission_repo.clone()
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

impl FromRef<AppState> for Arc<crate::integrations::public_site::PublicSiteNotifier> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.public_site_notifier.clone()
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

impl FromRef<AppState> for Arc<crate::service::member_field_service::MemberFieldService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.member_field_service.clone()
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

impl FromRef<AppState> for Arc<EventRegistrationService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.event_registration_service.clone()
    }
}

impl FromRef<AppState> for Arc<SeriesEnrollmentService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.series_enrollment_service.clone()
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

impl FromRef<AppState> for Arc<SubmissionService> {
    fn from_ref(state: &AppState) -> Self {
        state.service_context.submission_service.clone()
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

// Read the CURRENT Stripe wiring per request. FromRef runs on each
// extraction, so a `rebuild()` after a settings save is picked up by
// the next request with no restart.
impl FromRef<AppState> for Option<Arc<StripeClient>> {
    fn from_ref(state: &AppState) -> Self {
        state.stripe.current().client.clone()
    }
}

impl FromRef<AppState> for Option<Arc<WebhookDispatcher>> {
    fn from_ref(state: &AppState) -> Self {
        state.stripe.current().webhook_dispatcher.clone()
    }
}

impl FromRef<AppState> for Arc<StripeHandle> {
    fn from_ref(state: &AppState) -> Self {
        state.stripe.clone()
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
// RateLimiter appears twice on AppState (recovery_limiter,
// money_limiter), so a bare `FromRef<AppState> for RateLimiter` would be
// ambiguous. Each limiter gets a newtype wrapper.

#[derive(Clone)]
pub struct LoginLimiter(pub CredentialLimiter);

/// Password-recovery budget. Deliberately NOT `LoginLimiter` — see the
/// `recovery_limiter` field comment.
#[derive(Clone)]
pub struct RecoveryLimiter(pub RateLimiter);

#[derive(Clone)]
pub struct MoneyLimiter(pub RateLimiter);

impl FromRef<AppState> for LoginLimiter {
    fn from_ref(state: &AppState) -> Self {
        LoginLimiter(state.login_limiter.clone())
    }
}

impl FromRef<AppState> for RecoveryLimiter {
    fn from_ref(state: &AppState) -> Self {
        RecoveryLimiter(state.recovery_limiter.clone())
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    fn headers(xff: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", xff.parse().unwrap());
        h
    }

    #[test]
    fn xff_uses_rightmost_entry_not_client_prepended() {
        // Caddy appends the real peer (9.9.9.9); 1.2.3.4 is a client-supplied
        // prefix an attacker can rotate. We MUST key on the trusted-proxy hop.
        let ip = client_ip(&headers("1.2.3.4, 9.9.9.9"), true);
        assert_eq!(ip, "9.9.9.9".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn xff_single_entry_still_used() {
        let ip = client_ip(&headers("9.9.9.9"), true);
        assert_eq!(ip, "9.9.9.9".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn xff_ignored_when_not_trusted() {
        // Safe-by-default: header ignored, collapses to loopback bucket.
        let ip = client_ip(&headers("1.2.3.4, 9.9.9.9"), false);
        assert_eq!(ip, IpAddr::from([127, 0, 0, 1]));
    }

    // -----------------------------------------------------------------
    // CredentialLimiter
    //
    // The venue and stuffing cases are asserted here rather than through
    // the HTTP surface because each failed login there costs a full
    // Argon2 hash, and these need dozens of attempts.
    // -----------------------------------------------------------------

    /// Production shape: 5 failures per account, 20 accounts per address.
    fn limiter() -> CredentialLimiter {
        CredentialLimiter::new(5, 20, Duration::from_secs(15 * 60))
    }

    fn venue() -> IpAddr {
        "203.0.113.7".parse().unwrap()
    }

    #[test]
    fn successes_consume_nothing() {
        let limiter = limiter();
        // A hundred logins that never call record_failure — including
        // the same submission twice, which is the double-submit that
        // needs no de-duplication once successes are free.
        for _ in 0..100 {
            assert!(limiter.check(venue(), "admin@example.com", "auth.login"));
            assert!(limiter.check(venue(), "admin@example.com", "auth.totp"));
        }
    }

    #[test]
    fn sixth_failure_against_one_account_is_rejected() {
        let limiter = limiter();
        for _ in 0..5 {
            assert!(limiter.check(venue(), "admin@example.com", "auth.login"));
            limiter.record_failure(venue(), "admin@example.com");
        }
        assert!(!limiter.check(venue(), "admin@example.com", "auth.login"));
    }

    #[test]
    fn one_member_locking_themselves_out_does_not_reach_the_address_budget() {
        let limiter = limiter();
        // The venue: four members fumbling behind one NAT address, one
        // of them burning their whole budget.
        for _ in 0..5 {
            limiter.record_failure(venue(), "unlucky@example.com");
        }
        for who in ["b@example.com", "c@example.com", "d@example.com"] {
            limiter.record_failure(venue(), who);
            limiter.record_failure(venue(), who);
        }

        assert!(
            !limiter.check(venue(), "unlucky@example.com", "auth.login"),
            "the member who spent their own budget is locked out of login"
        );
        for who in ["b@example.com", "c@example.com", "e@example.com"] {
            assert!(
                limiter.check(venue(), who, "auth.login"),
                "{who} shares only an address with them, not a budget"
            );
        }
    }

    #[test]
    fn breadth_across_many_accounts_trips_the_address_budget() {
        let limiter = limiter();
        // One try each against 20 distinct accounts: no account is
        // anywhere near its own limit, but the spread is the signature
        // of stuffing rather than of people who know their passwords.
        for i in 0..20 {
            let who = format!("victim{i}@example.com");
            assert!(limiter.check(venue(), &who, "auth.login"));
            limiter.record_failure(venue(), &who);
        }
        assert!(!limiter.check(venue(), "victim99@example.com", "auth.login"));
        assert!(
            !limiter.check(venue(), "victim0@example.com", "auth.login"),
            "the address is over budget regardless of which account is named"
        );
        assert!(
            limiter.check(
                "198.51.100.1".parse().unwrap(),
                "victim0@example.com",
                "auth.login"
            ),
            "and another address is unaffected"
        );
    }

    #[test]
    fn identifiers_matching_no_member_count_toward_breadth() {
        // The limiter never resolves identifiers, so a run of names that
        // match nothing counts exactly as much as real ones — which is
        // the point: a source guessing at accounts that do not exist is
        // not a member mistyping their own password.
        let limiter = limiter();
        for i in 0..20 {
            limiter.record_failure(venue(), &format!("no-such-user-{i}"));
        }
        assert!(!limiter.check(venue(), "real@example.com", "auth.login"));
    }

    #[test]
    fn capitalization_and_padding_do_not_buy_a_fresh_budget() {
        let limiter = limiter();
        for variant in [
            "Admin@Example.com",
            " admin@example.com ",
            "ADMIN@EXAMPLE.COM",
            "admin@example.com",
            "aDmIn@ExAmPlE.cOm",
        ] {
            assert!(limiter.check(venue(), variant, "auth.login"));
            limiter.record_failure(venue(), variant);
        }
        assert!(!limiter.check(venue(), "admin@example.com", "auth.login"));
        assert_eq!(
            CredentialLimiter::account_key(" Admin@Example.com "),
            "admin@example.com"
        );
    }

    #[test]
    fn a_short_window_expires_both_budgets() {
        let limiter = CredentialLimiter::new(1, 1, Duration::from_millis(30));
        limiter.record_failure(venue(), "admin@example.com");
        assert!(!limiter.check(venue(), "admin@example.com", "auth.login"));
        std::thread::sleep(Duration::from_millis(45));
        assert!(limiter.check(venue(), "admin@example.com", "auth.login"));
    }

    #[test]
    fn cleanup_drops_expired_entries() {
        let limiter = CredentialLimiter::new(5, 20, Duration::from_millis(30));
        limiter.record_failure(venue(), "admin@example.com");
        std::thread::sleep(Duration::from_millis(45));
        limiter.cleanup();
        assert!(lock_recovering(&limiter.account_failures).is_empty());
        assert!(lock_recovering(&limiter.address_breadth).is_empty());
    }
}
