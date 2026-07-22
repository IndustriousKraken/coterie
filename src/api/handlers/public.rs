use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    api::{middleware::bot_challenge::BotChallengeVerifier, state::MoneyLimiter},
    auth::AuthService,
    config::Settings,
    domain::{
        configurable_types::MembershipTypeConfig, Announcement, AnnouncementType,
        CreateMemberRequest, Event, EventType, EventVisibility, MemberStatus, PaymentStatus,
    },
    email::EmailSender,
    error::{AppError, Result},
    payments::StripeClient,
    repository::{
        AnnouncementRepository, DonationCampaignRepository, EventRepository, MemberRepository,
        PaymentRepository,
    },
    service::{
        membership_type_service::MembershipTypeService,
        settings_service::{stripe_keys, SettingsService, SignupMode},
    },
};

#[derive(Debug, Deserialize, ToSchema)]
pub struct SignupRequest {
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub password: String,
    /// Slug of the membership type to assign (e.g. `member`,
    /// `student`). Omit to take the org's default — the first
    /// `is_active` row in `membership_types` ordered by `sort_order`.
    pub membership_type_slug: Option<String>,
    /// Bot-challenge token from the marketing site's CAPTCHA widget.
    /// Required when the org has configured a provider; ignored when
    /// `bot_challenge.provider = "disabled"`. See `BotChallengeConfig`.
    #[serde(default)]
    pub captcha_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SignupResponse {
    pub member_id: Uuid,
    pub status: MemberStatus,
    pub message: String,
    /// Present only when the org's signup mode is `payment` and the
    /// chosen membership type has a fee: redirect the browser here to
    /// complete the Stripe Checkout that activates the membership.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkout_url: Option<String>,
}

/// Public projection of a membership type for the join form. Deliberately
/// excludes internal fields (`id`, `is_active`, timestamps) — the slug is
/// the public identifier and is what `POST /public/signup` accepts.
#[derive(Debug, Serialize, ToSchema)]
pub struct PublicMembershipType {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub fee_cents: i32,
    pub currency: String,
    /// One of `monthly`, `yearly`, `lifetime`.
    pub billing_period: String,
}

/// Public projection of an `Event` for `GET /public/events`. Exposes
/// only the fields the marketing site consumes and deliberately omits
/// internal identifiers that must never reach anonymous callers —
/// `created_by` (the organizer's member id), `created_at`, `updated_at`,
/// `event_type_id`, `series_id`, and `occurrence_index`. Members-only
/// sanitization (nulling title/description/location/image_url) is applied
/// to the source `Event` before projection, so it carries through.
#[derive(Debug, Serialize, ToSchema)]
pub struct PublicEvent {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub event_type: EventType,
    pub visibility: EventVisibility,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub timezone: String,
    pub location: Option<String>,
    pub image_url: Option<String>,
    pub max_attendees: Option<i32>,
    pub rsvp_required: bool,
}

impl From<Event> for PublicEvent {
    fn from(e: Event) -> Self {
        PublicEvent {
            id: e.id,
            title: e.title,
            description: e.description,
            event_type: e.event_type,
            visibility: e.visibility,
            start_time: e.start_time,
            end_time: e.end_time,
            timezone: e.timezone,
            location: e.location,
            image_url: e.image_url,
            max_attendees: e.max_attendees,
            rsvp_required: e.rsvp_required,
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct PublicEventsQuery {
    /// Maximum number of events to return (default 50).
    pub limit: Option<i64>,
    /// Response format: omit or `"json"` for JSON; `"ical"` for an
    /// iCal/.ics calendar feed.
    pub format: Option<String>,
    /// Optional inclusive start of a date range (RFC 3339 instant). When
    /// both `from` and `to` are supplied, valid, `to > from`, and the span
    /// is within the maximum window, the JSON feed returns events whose
    /// derived UTC instant is in `[from, to)` — **including past events** —
    /// instead of the default upcoming-only list. Ignored for `format=ical`
    /// and silently ignored (falls back to upcoming-only) if malformed.
    pub from: Option<String>,
    /// Optional exclusive end of the date range (RFC 3339 instant). See `from`.
    pub to: Option<String>,
}

/// Maximum span of a `from`/`to` range on `GET /public/events`. Bounds the
/// scan an anonymous caller can request; a wider (or malformed) range falls
/// back to the default upcoming-only list. ~400 days covers a full calendar
/// month view (with adjacent-month spill) plus slack.
const MAX_RANGE_SPAN_DAYS: i64 = 400;

/// Parse the opt-in `from`/`to` range. Returns `Some((from, to))` only when
/// BOTH parse as RFC 3339 instants, `to > from`, and the window is no wider
/// than `MAX_RANGE_SPAN_DAYS`; otherwise `None`, so the caller falls back to
/// the default upcoming-only filter (a bad range must never error).
fn parse_range(
    from: &Option<String>,
    to: &Option<String>,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let from = DateTime::parse_from_rfc3339(from.as_deref()?)
        .ok()?
        .with_timezone(&Utc);
    let to = DateTime::parse_from_rfc3339(to.as_deref()?)
        .ok()?
        .with_timezone(&Utc);
    (to > from && to - from <= Duration::days(MAX_RANGE_SPAN_DAYS)).then_some((from, to))
}

#[utoipa::path(
    post,
    path = "/public/signup",
    tag = "public",
    request_body = SignupRequest,
    responses(
        (status = 201, description = "Member created; verification email sent. In payment \
            mode, `checkout_url` carries the Stripe Checkout link that completes (and \
            activates) the membership", body = SignupResponse),
        (status = 200, description = "Payment-mode retry: the email belongs to a Pending \
            signup with no completed payment and the password verified — a fresh \
            `checkout_url` is returned instead of a duplicate error", body = SignupResponse),
        (status = 400, description = "Invalid email or weak password"),
        (status = 409, description = "Email or username already in use"),
        (status = 429, description = "Rate limited (per-IP money limiter, both signup modes)"),
    ),
)]
pub async fn signup(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(bot_challenge_verifier): State<Arc<dyn BotChallengeVerifier>>,
    State(email_sender): State<Arc<dyn EmailSender>>,
    State(settings): State<Arc<Settings>>,
    State(settings_service): State<Arc<SettingsService>>,
    State(db_pool): State<SqlitePool>,
    State(money_limiter): State<MoneyLimiter>,
    State(stripe_client): State<Option<Arc<StripeClient>>>,
    State(payment_repo): State<Arc<dyn PaymentRepository>>,
    headers: HeaderMap,
    Json(request): Json<SignupRequest>,
) -> Result<(StatusCode, Json<SignupResponse>)> {
    let signup_mode = settings_service.signup_mode().await;
    let ip = crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for());

    // Rate limit FIRST (before the bot challenge, so a bursting IP can't
    // burn the provider's quota), mirroring /public/donate. Applied in
    // BOTH modes: payment mode caps card-testing on the Stripe Checkout
    // side-effect; approval mode caps mass account creation and the
    // verification-email amplification each signup triggers — the bot
    // challenge defaults to disabled, so this is the only remaining
    // control out of the box.
    if !money_limiter.0.check_and_record(ip) {
        return Err(AppError::TooManyRequests);
    }

    // Bot-challenge verification BEFORE any work. Fail closed: if the
    // org has configured a provider, every request must carry a token
    // the provider verifies. The DisabledVerifier is a no-op so dev
    // setups don't break.
    if bot_challenge_verifier
        .verify("public/signup", request.captcha_token.as_deref(), Some(ip))
        .await
        .is_err()
    {
        return Err(AppError::Forbidden);
    }

    // Bound and validate the free-text fields before persisting. This
    // is an unauthenticated endpoint, so unbounded/empty input is a
    // storage-abuse vector. Caps mirror the public-donate handler.
    let email = request.email.trim();
    let username = request.username.trim();
    let full_name = request.full_name.trim();
    if email.is_empty() || !email.contains('@') {
        return Err(AppError::BadRequest("Valid email is required".to_string()));
    }
    if email.len() > 254 {
        return Err(AppError::BadRequest("Email too long".to_string()));
    }
    if full_name.is_empty() {
        return Err(AppError::BadRequest("Full name is required".to_string()));
    }
    if full_name.len() > 200 {
        return Err(AppError::BadRequest("Full name too long".to_string()));
    }
    if username.is_empty() {
        return Err(AppError::BadRequest("Username is required".to_string()));
    }
    if username.len() > 100 {
        return Err(AppError::BadRequest("Username too long".to_string()));
    }

    // Validate password strength
    if let Err(msg) = crate::auth::validate_password(&request.password) {
        return Err(AppError::BadRequest(msg.to_string()));
    }

    // Resolve the requested membership_type slug to an FK. Unknown
    // slugs fail loudly (BadRequest) — silently mapping to a default
    // would mask client typos.
    let membership_type_id = match request.membership_type_slug.as_deref() {
        Some(slug) => {
            let mt = membership_type_service
                .get_by_slug(slug)
                .await?
                .ok_or_else(|| {
                    AppError::BadRequest(format!("Unknown membership type slug: {}", slug,))
                })?;
            // A deactivated type is not signup-able. Reject it with the
            // same 400 AND the same message as an unknown slug, before any
            // member is created — inactive types are excluded from the public
            // listing, and using an identical message keeps a prober from
            // distinguishing "never existed" from "exists but deactivated".
            if !mt.is_active {
                return Err(AppError::BadRequest(format!(
                    "Unknown membership type slug: {}",
                    slug,
                )));
            }
            Some(mt.id)
        }
        None => None,
    };

    // Create member with Pending status
    let create_request = CreateMemberRequest {
        email: email.to_string(),
        username: username.to_string(),
        full_name: full_name.to_string(),
        password: request.password.clone(),
        membership_type_id,
        ..Default::default()
    };

    // Create the member. Use a generic error for UNIQUE violations to
    // prevent attackers from enumerating valid emails/usernames.
    let member = match member_repo.create(create_request).await {
        Ok(m) => m,
        Err(e) => {
            let is_unique_violation = matches!(
                &e,
                AppError::Database(sqlx::Error::Database(db_err)) if db_err.is_unique_violation()
            );
            if !is_unique_violation {
                return Err(e);
            }
            // Payment-mode abandoned-checkout retry: an existing
            // Pending member with no completed membership payment who
            // proves the password gets a fresh checkout session instead
            // of being stranded (Pending can't log in to pay, duplicate
            // email can't re-signup). Every other duplicate — wrong
            // password, already paid, non-Pending — falls through to
            // the exact pre-existing generic outcome so this path
            // discloses nothing duplicate detection doesn't already.
            if signup_mode == SignupMode::Payment {
                if let Some(response) = retry_pending_checkout(
                    member_repo.as_ref(),
                    payment_repo.as_ref(),
                    &membership_type_service,
                    stripe_client.as_deref(),
                    &settings_service,
                    &settings,
                    &db_pool,
                    email,
                    &request.password,
                )
                .await?
                {
                    return Ok(response);
                }
            }
            return Err(AppError::Conflict(
                "Registration failed: an account with this information already exists"
                    .to_string(),
            ));
        }
    };

    // Send email verification. Soft-fail on send error: the account is
    // already created and an admin can manually verify / resend later.
    if let Err(e) = send_verification_email(
        &db_pool,
        &settings,
        &settings_service,
        email_sender.as_ref(),
        &member,
    )
    .await
    {
        tracing::error!(
            "Signup succeeded but verification email failed for member {}: {}",
            member.id,
            e
        );
    }

    // Payment mode with a paid type: hand back a Stripe Checkout URL.
    // The completed payment (via webhook) extends dues AND activates
    // the Pending member. Fee-0 types stay in the approval funnel. A
    // checkout-creation failure is soft: the member + verification
    // email already exist, the retry path can mint a fresh session,
    // and an admin can always activate manually.
    let mut checkout_url = None;
    if signup_mode == SignupMode::Payment {
        if let Some(mt) = membership_type_service
            .get(member.membership_type_id)
            .await?
        {
            if mt.fee_cents > 0 {
                match stripe_client.as_deref() {
                    Some(client) => {
                        match create_signup_checkout(
                            client,
                            &settings_service,
                            &settings,
                            &member,
                            &mt,
                        )
                        .await
                        {
                            Ok(url) => checkout_url = Some(url),
                            Err(e) => tracing::error!(
                                "Signup checkout creation failed for member {}: {}",
                                member.id,
                                e,
                            ),
                        }
                    }
                    None => tracing::error!(
                        "signup_mode=payment but Stripe is not configured; member {} \
                         created Pending without a checkout session",
                        member.id,
                    ),
                }
            }
        }
    }

    let message = if checkout_url.is_some() {
        "Registration successful. Complete your membership payment at the checkout link; \
         a verification email is also on its way."
            .to_string()
    } else {
        "Registration successful. Please check your email to verify your account.".to_string()
    };

    let response = SignupResponse {
        member_id: member.id,
        status: member.status,
        message,
        checkout_url,
    };

    Ok((StatusCode::CREATED, Json(response)))
}

/// Success/cancel URLs for a signup checkout: the operator-set
/// `stripe.success_url` / `stripe.cancel_url` settings when non-blank
/// (point these at the marketing site's welcome/cancel pages), else the
/// portal payment pages.
async fn signup_checkout_urls(
    settings_service: &SettingsService,
    settings: &Settings,
) -> (String, String) {
    let nonblank = |v: crate::error::Result<String>| v.ok().filter(|s| !s.trim().is_empty());
    let success = nonblank(settings_service.get_value(stripe_keys::SUCCESS_URL).await)
        .unwrap_or_else(|| format!("{}/portal/payments/success", settings.server.base_url));
    let cancel = nonblank(settings_service.get_value(stripe_keys::CANCEL_URL).await)
        .unwrap_or_else(|| format!("{}/portal/payments/cancel", settings.server.base_url));
    (success, cancel)
}

/// Create the Stripe Checkout session for a signup (fresh or retried).
/// Reuses the portal dues-checkout contract — the session metadata
/// (`member_id`, `payment_type=membership`, `membership_type_slug`) is
/// exactly what the webhook path already honors to record the payment,
/// extend dues, and activate the member.
///
/// When `membership.signup_auto_renew` is on (the default), the session
/// is bound to a Stripe customer for the member with the card saved
/// off-session and `save_card=true` metadata — the completed-checkout
/// webhook keys auto-renew enrollment on that stamp.
async fn create_signup_checkout(
    stripe_client: &StripeClient,
    settings_service: &SettingsService,
    settings: &Settings,
    member: &crate::domain::Member,
    membership_type: &MembershipTypeConfig,
) -> Result<String> {
    let save_card = settings_service.signup_auto_renew().await;
    let customer_id = if save_card {
        // Soft-fail to a one-off session: a customer-creation hiccup
        // must not block the signup payment itself.
        match stripe_client
            .get_or_create_customer(member.id, &member.email, &member.full_name)
            .await
        {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::error!(
                    "Signup auto-renew: customer creation failed for member {} \
                     (falling back to one-off checkout): {}",
                    member.id,
                    e,
                );
                None
            }
        }
    } else {
        None
    };
    let save_card = save_card && customer_id.is_some();

    let (success_url, cancel_url) = signup_checkout_urls(settings_service, settings).await;
    let (url, _payment_id) = stripe_client
        .create_membership_checkout_session(
            member.id,
            &membership_type.name,
            &membership_type.slug,
            membership_type.fee_cents as i64,
            success_url,
            cancel_url,
            customer_id,
            save_card,
        )
        .await?;
    Ok(url)
}

/// The payment-mode duplicate-signup retry (see the pay-at-signup
/// spec): returns `Some(response)` with a fresh checkout URL only when
/// ALL of these hold — the email belongs to a Pending member with no
/// completed membership payment, the supplied password verifies against
/// that member's hash, Stripe is configured, and their membership type
/// has a fee. Any other case returns `None` and the caller emits the
/// exact pre-existing generic duplicate outcome.
#[allow(clippy::too_many_arguments)]
async fn retry_pending_checkout(
    member_repo: &dyn MemberRepository,
    payment_repo: &dyn PaymentRepository,
    membership_type_service: &MembershipTypeService,
    stripe_client: Option<&StripeClient>,
    settings_service: &SettingsService,
    settings: &Settings,
    db_pool: &SqlitePool,
    email: &str,
    password: &str,
) -> Result<Option<(StatusCode, Json<SignupResponse>)>> {
    let Some(member) = member_repo.find_by_email(email).await? else {
        // Unique violation on username, not email — not a retry. Burn the
        // same Argon2 time the verify branch does so the 409's latency
        // can't distinguish a Pending-unpaid email from other outcomes.
        AuthService::verify_dummy(password).await;
        return Ok(None);
    };
    if member.status != MemberStatus::Pending {
        AuthService::verify_dummy(password).await;
        return Ok(None);
    }
    let Some(hash) = crate::auth::get_password_hash(db_pool, email).await? else {
        AuthService::verify_dummy(password).await;
        return Ok(None);
    };
    if !AuthService::verify_password(password, &hash)
        .await
        .unwrap_or(false)
    {
        return Ok(None);
    }
    let payments = payment_repo.find_by_member(member.id).await?;
    let has_completed_membership_payment = payments.iter().any(|p| {
        matches!(p.kind, crate::domain::PaymentKind::Membership)
            && p.status == PaymentStatus::Completed
    });
    if has_completed_membership_payment {
        return Ok(None);
    }
    let Some(stripe_client) = stripe_client else {
        return Ok(None);
    };
    let Some(membership_type) = membership_type_service
        .get(member.membership_type_id)
        .await?
    else {
        return Ok(None);
    };
    if membership_type.fee_cents <= 0 {
        return Ok(None);
    }

    let retry_message =
        "Welcome back — complete your membership payment at the checkout link.".to_string();

    // Prefer resuming the existing OPEN session over minting another:
    // a retry must not accumulate duplicate pending payment rows or
    // leave several payable sessions open at once. `find_by_member` is
    // newest-first, so the first pending checkout row is the latest.
    // Any session found no-longer-open is superseded: its Pending row
    // flips to Failed so the ledger reads truthfully.
    let pending_sessions: Vec<(Uuid, String)> = payments
        .iter()
        .filter(|p| {
            p.status == PaymentStatus::Pending
                && matches!(p.kind, crate::domain::PaymentKind::Membership)
        })
        .filter_map(|p| match &p.external_id {
            Some(crate::domain::StripeRef::CheckoutSession(sid)) => Some((p.id, sid.clone())),
            _ => None,
        })
        .collect();
    for (payment_id, session_id) in &pending_sessions {
        match stripe_client
            .gateway()
            .retrieve_checkout_session(session_id)
            .await
        {
            Ok(session) if session.is_open => {
                if let Some(url) = session.url {
                    return Ok(Some((
                        StatusCode::OK,
                        Json(SignupResponse {
                            member_id: member.id,
                            status: member.status,
                            message: retry_message,
                            checkout_url: Some(url),
                        }),
                    )));
                }
            }
            Ok(_) => {
                // Expired or completed-without-our-webhook-yet: mark the
                // stale pending row Failed before minting a replacement.
                let _ = payment_repo.fail_pending_payment(*payment_id).await;
            }
            Err(e) => {
                tracing::warn!(
                    "Signup retry: could not retrieve checkout session {} for member {}: {}",
                    session_id,
                    member.id,
                    e,
                );
            }
        }
    }

    let url = create_signup_checkout(
        stripe_client,
        settings_service,
        settings,
        &member,
        &membership_type,
    )
    .await?;

    Ok(Some((
        StatusCode::OK,
        Json(SignupResponse {
            member_id: member.id,
            status: member.status,
            message: retry_message,
            checkout_url: Some(url),
        }),
    )))
}

/// Generate a verification token and email the link to the member.
async fn send_verification_email(
    db_pool: &SqlitePool,
    settings: &Settings,
    settings_service: &SettingsService,
    email_sender: &dyn EmailSender,
    member: &crate::domain::Member,
) -> Result<()> {
    use crate::{
        auth,
        email::{
            self,
            templates::{VerifyHtml, VerifyText},
        },
    };

    let created = auth::email_tokens::create_verification_token(
        db_pool,
        member.id,
        chrono::Duration::hours(24),
    )
    .await?;

    let verify_url = format!(
        "{}/verify?token={}",
        settings.server.base_url.trim_end_matches('/'),
        created.token,
    );
    let org_name = org_name(settings_service).await;
    let html = VerifyHtml {
        full_name: &member.full_name,
        org_name: &org_name,
        verify_url: &verify_url,
    };
    let text = VerifyText {
        full_name: &member.full_name,
        org_name: &org_name,
        verify_url: &verify_url,
    };
    let message = email::message_from_templates(
        member.email.clone(),
        format!("Verify your email for {}", org_name),
        &html,
        &text,
    )?;
    email_sender.send(&message).await
}

/// Look up the configured organization name from settings, falling back
/// to "Coterie" if unset.
async fn org_name(settings_service: &SettingsService) -> String {
    settings_service
        .get_value("org.name")
        .await
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Coterie".to_string())
}

#[utoipa::path(
    get,
    path = "/public/events",
    tag = "public",
    params(PublicEventsQuery),
    responses(
        (status = 200, description = "Upcoming public + sanitized members-only events", body = [PublicEvent],
            content_type = "application/json"),
        (status = 200, description = "iCal feed (when format=ical)", content_type = "text/calendar"),
    ),
)]
pub async fn list_events(
    State(event_repo): State<Arc<dyn EventRepository>>,
    Query(params): Query<PublicEventsQuery>,
) -> Result<Response> {
    // Get public events (full details)
    let public_events = event_repo.list_public().await?;

    // Get members-only events (will be sanitized)
    let private_events = event_repo.list_members_only().await?;

    // Combine, then replace each event's stored wall-clock with its
    // derived UTC instant so the "upcoming" filter, the sort, and the
    // JSON/iCal output all compare and emit true instants (not the
    // naive wall-clock, which would be off by the org's offset).
    let now = Utc::now();
    let mut events: Vec<Event> = public_events
        .into_iter()
        .chain(private_events.into_iter().map(|mut e| {
            // Sanitize private events
            e.title = "Members-Only Event".to_string();
            e.description =
                "This event is for members only. Log in to the portal to see details.".to_string();
            e.location = None;
            e.image_url = None;
            e
        }))
        .collect();
    // `start_time`/`end_time` now hold the derived UTC instant, so the
    // filters below compare true instants (not the naive wall-clock).
    derive_utc_instants(&mut events);

    // iCal is ALWAYS upcoming-only (the home page + calendar subscriptions
    // depend on it); the range opt-in applies to the JSON feed only. A range
    // is honored only when both `from`/`to` parse, `to > from`, and the span
    // is bounded — otherwise we fall back to the upcoming filter unchanged.
    let is_ical = params.format.as_deref() == Some("ical");
    let range = if is_ical {
        None
    } else {
        parse_range(&params.from, &params.to)
    };
    match range {
        Some((from, to)) => events.retain(|e| e.start_time >= from && e.start_time < to),
        None => events.retain(|e| e.start_time > now),
    }

    // Sort by start time
    events.sort_by(|a, b| a.start_time.cmp(&b.start_time));

    // Apply limit
    events.truncate(params.limit.unwrap_or(50) as usize);

    if is_ical {
        let ical = generate_ical_feed(&events);
        Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/calendar; charset=utf-8")],
            ical,
        )
            .into_response())
    } else {
        // Project to PublicEvent so internal-only fields (created_by,
        // timestamps, event_type_id, series_id, occurrence_index) never
        // reach anonymous callers. Sanitization already ran above.
        let public: Vec<PublicEvent> = events.into_iter().map(PublicEvent::from).collect();
        Ok(Json(public).into_response())
    }
}

/// Public projection of an `Announcement` for `GET /public/announcements`.
/// Exposes only the fields the marketing site consumes and deliberately
/// omits internal identifiers/implementation detail that must never reach
/// anonymous callers — `created_by` (the author's member id), `created_at`,
/// `updated_at`, `announcement_type_id`, `is_public`, and the scheduling
/// fields (`scheduled_publish_at`, `scheduled_publish_timezone`). Mirrors
/// `PublicEvent`.
///
/// Alongside the raw Markdown `content` it carries a server-rendered
/// sanitized `content_html` (Markdown → safe-subset HTML) so a consumer can
/// render formatted content without running its own Markdown parser or
/// making a sanitization decision.
#[derive(Debug, Serialize, ToSchema)]
pub struct PublicAnnouncement {
    pub id: Uuid,
    pub title: String,
    /// Raw Markdown source.
    pub content: String,
    /// Server-rendered sanitized safe-subset HTML of `content`.
    pub content_html: String,
    pub announcement_type: AnnouncementType,
    pub featured: bool,
    pub image_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}

impl From<Announcement> for PublicAnnouncement {
    fn from(a: Announcement) -> Self {
        PublicAnnouncement {
            content_html: crate::util::markdown::render_announcement_markdown(&a.content),
            id: a.id,
            title: a.title,
            content: a.content,
            announcement_type: a.announcement_type,
            featured: a.featured,
            image_url: a.image_url,
            published_at: a.published_at,
        }
    }
}

#[utoipa::path(
    get,
    path = "/public/announcements",
    tag = "public",
    responses(
        (status = 200, description = "Published public announcements, each with a \
            server-rendered sanitized `content_html` alongside the raw Markdown `content`",
            body = [PublicAnnouncement]),
    ),
)]
pub async fn list_announcements(
    State(announcement_repo): State<Arc<dyn AnnouncementRepository>>,
) -> Result<Json<Vec<PublicAnnouncement>>> {
    // Get public announcements only
    let announcements = announcement_repo.list_public().await?;

    // Filter to published announcements only, then project to
    // PublicAnnouncement so internal-only fields (created_by, timestamps,
    // announcement_type_id, is_public, scheduled_publish_*) never reach
    // anonymous callers. The projection also attaches the sanitized
    // server-rendered HTML from the shared Markdown pipeline.
    let published: Vec<PublicAnnouncement> = announcements
        .into_iter()
        .filter(|a| a.published_at.is_some())
        .map(PublicAnnouncement::from)
        .collect();

    Ok(Json(published))
}

#[utoipa::path(
    get,
    path = "/public/membership-types",
    tag = "public",
    responses(
        (status = 200, description = "Active membership types, ordered by sort_order, \
            for the public join form", body = [PublicMembershipType]),
    ),
)]
pub async fn list_membership_types(
    State(membership_type_service): State<Arc<MembershipTypeService>>,
) -> Result<Json<Vec<PublicMembershipType>>> {
    // Active types only; the repo already orders by sort_order, name.
    let types = membership_type_service.list(false).await?;
    Ok(Json(
        types
            .into_iter()
            .map(|t| PublicMembershipType {
                slug: t.slug,
                name: t.name,
                description: t.description,
                fee_cents: t.fee_cents,
                // Payments are USD throughout (see stripe_client); emit it
                // explicitly so the form can render without assuming.
                currency: "USD".to_string(),
                billing_period: t.billing_period,
            })
            .collect(),
    ))
}

#[utoipa::path(
    get,
    path = "/public/feed/rss",
    tag = "public",
    responses(
        (status = 200, description = "RSS 2.0 feed of public announcements",
            content_type = "application/rss+xml"),
    ),
)]
pub async fn rss_feed(
    State(announcement_repo): State<Arc<dyn AnnouncementRepository>>,
) -> Result<Response> {
    // Get recent public announcements
    let announcements = announcement_repo.list_public().await?;

    // Generate RSS XML
    let rss = generate_rss_feed(&announcements);

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/rss+xml; charset=utf-8")],
        rss,
    )
        .into_response())
}

#[utoipa::path(
    get,
    path = "/public/feed/calendar",
    tag = "public",
    responses(
        (status = 200, description = "iCal feed of all events (private events are sanitized)",
            content_type = "text/calendar"),
    ),
)]
pub async fn calendar_feed(State(event_repo): State<Arc<dyn EventRepository>>) -> Result<Response> {
    // Get public events (full details)
    let public_events = event_repo.list_public().await?;

    // Get members-only events (will be sanitized in feed)
    let private_events = event_repo.list_members_only().await?;

    // Combine all events for the calendar, deriving the UTC instant for
    // each from its (wall-clock, zone) before emitting.
    let mut all_events: Vec<_> = public_events
        .into_iter()
        .chain(private_events.into_iter())
        .collect();
    derive_utc_instants(&mut all_events);

    // Generate iCal format (private events will be sanitized)
    let ical = generate_ical_feed(&all_events);

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/calendar; charset=utf-8")],
        ical,
    )
        .into_response())
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PrivateEventCount {
    pub count: i64,
}

#[utoipa::path(
    get,
    path = "/public/events/private-count",
    tag = "public",
    responses(
        (status = 200, description = "Count of upcoming members-only events", body = PrivateEventCount),
    ),
)]
pub async fn private_event_count(
    State(event_repo): State<Arc<dyn EventRepository>>,
) -> Result<Json<PrivateEventCount>> {
    let count = event_repo.count_members_only_upcoming().await?;
    Ok(Json(PrivateEventCount { count }))
}

/// Escape text for use inside XML CDATA sections. The only sequence that
/// can break a CDATA block is `]]>`, which we split into two adjacent
/// CDATA sections: `]]]]><![CDATA[>`.
fn escape_cdata(s: &str) -> String {
    s.replace("]]>", "]]]]><![CDATA[>")
}

// Helper function to generate RSS feed
fn generate_rss_feed(announcements: &[Announcement]) -> String {
    let mut rss = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom">
<channel>
    <title>Coterie Announcements</title>
    <link>https://example.com/announcements</link>
    <description>Latest announcements from Coterie</description>
    <language>en-us</language>
    <lastBuildDate>"#,
    );

    rss.push_str(&Utc::now().to_rfc2822());
    rss.push_str("</lastBuildDate>\n");

    for announcement in announcements.iter().take(20) {
        if let Some(published) = announcement.published_at {
            rss.push_str("    <item>\n");
            rss.push_str(&format!(
                "        <title><![CDATA[{}]]></title>\n",
                escape_cdata(&announcement.title)
            ));
            // Description carries the sanitized rendered HTML (Markdown →
            // safe subset), CDATA-wrapped — valid RSS 2.0. escape_cdata
            // still guards any `]]>` the rendered HTML might contain.
            let content_html =
                crate::util::markdown::render_announcement_markdown(&announcement.content);
            rss.push_str(&format!(
                "        <description><![CDATA[{}]]></description>\n",
                escape_cdata(&content_html)
            ));
            rss.push_str(&format!(
                "        <guid isPermaLink=\"false\">{}</guid>\n",
                announcement.id
            ));
            rss.push_str(&format!(
                "        <pubDate>{}</pubDate>\n",
                published.to_rfc2822()
            ));
            rss.push_str("    </item>\n");
        }
    }

    rss.push_str("</channel>\n</rss>");
    rss
}

/// Escape a text value for iCal (RFC 5545 Section 3.3.11).
/// Backslashes, semicolons, commas, and newlines must be escaped.
fn escape_ical_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
        .replace('\r', "")
}

/// Replace each event's stored wall-clock `start_time`/`end_time` with
/// its derived UTC instant (from the event's IANA zone), in place. Once
/// applied, downstream serialization — the `…Z` JSON timestamps and the
/// iCal `DTSTART`/`DTEND` — emits correct instants without any further
/// per-call conversion. Idempotent only if called once per read path;
/// callers run it exactly once before filtering/sorting/serializing.
fn derive_utc_instants(events: &mut [Event]) {
    for e in events.iter_mut() {
        let start = e.start_utc();
        let end = e.end_utc();
        e.start_time = start;
        e.end_time = end;
    }
}

// Helper function to generate iCal feed
// Private (MembersOnly) events are sanitized to show only time slot
fn generate_ical_feed(events: &[Event]) -> String {
    let mut ical = String::from("BEGIN:VCALENDAR\r\n");
    ical.push_str("VERSION:2.0\r\n");
    ical.push_str("PRODID:-//Coterie//Events//EN\r\n");
    ical.push_str("CALSCALE:GREGORIAN\r\n");
    ical.push_str("METHOD:PUBLISH\r\n");
    ical.push_str("X-WR-CALNAME:Coterie Events\r\n");

    for event in events {
        let is_private = event.visibility != EventVisibility::Public;

        ical.push_str("BEGIN:VEVENT\r\n");
        ical.push_str(&format!("UID:{}\r\n", event.id));
        ical.push_str(&format!(
            "DTSTART:{}\r\n",
            event.start_time.format("%Y%m%dT%H%M%SZ")
        ));

        if let Some(end_time) = event.end_time {
            ical.push_str(&format!("DTEND:{}\r\n", end_time.format("%Y%m%dT%H%M%SZ")));
        }

        if is_private {
            // Sanitize private events - show only that something is happening
            ical.push_str("SUMMARY:Members-Only Event\r\n");
            ical.push_str("DESCRIPTION:This event is for members only. Log in to the portal to see details.\r\n");
        } else {
            ical.push_str(&format!("SUMMARY:{}\r\n", escape_ical_text(&event.title)));
            ical.push_str(&format!(
                "DESCRIPTION:{}\r\n",
                escape_ical_text(&event.description)
            ));

            if let Some(location) = &event.location {
                ical.push_str(&format!("LOCATION:{}\r\n", escape_ical_text(location)));
            }
        }

        ical.push_str(&format!(
            "CREATED:{}\r\n",
            event.created_at.format("%Y%m%dT%H%M%SZ")
        ));
        ical.push_str(&format!(
            "LAST-MODIFIED:{}\r\n",
            event.updated_at.format("%Y%m%dT%H%M%SZ")
        ));
        ical.push_str("STATUS:CONFIRMED\r\n");
        ical.push_str("END:VEVENT\r\n");
    }

    ical.push_str("END:VCALENDAR\r\n");
    ical
}

// ---------------------------------------------------------------------
// Public donation API
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct PublicDonateRequest {
    pub amount_cents: i64,
    pub email: String,
    pub name: String,
    /// Optional campaign slug. If absent or empty, the donation is
    /// recorded as a general donation (no campaign attribution).
    #[serde(default)]
    pub campaign_slug: Option<String>,
    /// Bot-challenge token from the marketing site's CAPTCHA widget.
    /// Required when the org has configured a provider; ignored when
    /// `bot_challenge.provider = "disabled"`. See `BotChallengeConfig`.
    #[serde(default)]
    pub captcha_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PublicDonateResponse {
    pub payment_id: Uuid,
    /// Stripe-hosted Checkout URL. The frontend redirects the donor here.
    pub checkout_url: String,
}

/// POST /public/donate — accepts a donation from a non-authenticated
/// donor and returns a Stripe Checkout URL to redirect them to.
///
/// Flow:
///   1. Validate amount + email + name + campaign-if-given
///   2. Check IP rate limit (money_limiter, 10/min/IP)
///   3. If donor's email matches an existing member → attach donation
///      to that member's payment history. Otherwise → record as a
///      public donation with donor_name + donor_email on the row.
///   4. Create Stripe Checkout session, return URL.
///   5. (Webhook side) When the session completes, the existing
///      payment_intent.succeeded / checkout.session.completed handlers
///      flip the row to Completed. Donations don't extend dues, so
///      there's no further bookkeeping.
///
/// CORS: same origin policy as other /public/* endpoints. The public
/// site (e.g. neontemple.net) is expected to be in
/// COTERIE__SERVER__CORS_ORIGINS.
#[utoipa::path(
    post,
    path = "/public/donate",
    tag = "public",
    request_body = PublicDonateRequest,
    responses(
        (status = 200, description = "Stripe Checkout session created; redirect donor to checkout_url",
            body = PublicDonateResponse),
        (status = 400, description = "Invalid amount, email, name, or campaign"),
        (status = 429, description = "Rate-limit hit (per-IP money limiter)"),
        (status = 503, description = "Payment processing not configured"),
    ),
)]
pub async fn donate(
    State(settings): State<Arc<Settings>>,
    State(money_limiter): State<MoneyLimiter>,
    State(bot_challenge_verifier): State<Arc<dyn BotChallengeVerifier>>,
    State(donation_campaign_repo): State<Arc<dyn DonationCampaignRepository>>,
    State(stripe_client): State<Option<Arc<StripeClient>>>,
    State(member_repo): State<Arc<dyn MemberRepository>>,
    headers: HeaderMap,
    Json(request): Json<PublicDonateRequest>,
) -> Result<(StatusCode, Json<PublicDonateResponse>)> {
    // Rate limit by client IP. Public endpoint with payment side-effects
    // is the prime card-testing target — the limiter caps each IP at
    // 10 attempts per minute. Rate limit BEFORE the bot challenge so
    // a bursting IP can't burn through the provider's quota.
    let ip = crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for());
    if !money_limiter.0.check_and_record(ip) {
        return Err(AppError::TooManyRequests);
    }

    // Bot-challenge verification BEFORE Stripe Checkout. Carding
    // attacks abuse this endpoint to test stolen cards; the per-IP
    // limiter stops single-source bursts but distributed bots roll
    // right past it. Fail closed when a provider is configured.
    if bot_challenge_verifier
        .verify("public/donate", request.captcha_token.as_deref(), Some(ip))
        .await
        .is_err()
    {
        return Err(AppError::Forbidden);
    }

    // Validation. Bounds match the logged-in donate flow.
    if request.amount_cents <= 0 {
        return Err(AppError::BadRequest("Amount must be positive".to_string()));
    }
    if request.amount_cents > crate::domain::MAX_PAYMENT_CENTS {
        return Err(AppError::BadRequest(format!(
            "Amount exceeds the ${} cap on a single donation",
            crate::domain::MAX_PAYMENT_CENTS / 100,
        )));
    }
    let email = request.email.trim();
    let name = request.name.trim();
    if email.is_empty() || !email.contains('@') {
        return Err(AppError::BadRequest("Valid email is required".to_string()));
    }
    if email.len() > 254 {
        return Err(AppError::BadRequest("Email too long".to_string()));
    }
    if name.is_empty() {
        return Err(AppError::BadRequest("Name is required".to_string()));
    }
    if name.len() > 200 {
        return Err(AppError::BadRequest("Name too long".to_string()));
    }

    // Resolve campaign. Same logic as the logged-in path: blank/missing
    // slug = general donation; unknown slug also = general (donor
    // shouldn't get a hard error for stale URL); inactive = reject.
    let (campaign_id, campaign_name) = match request.campaign_slug.as_deref() {
        Some(slug) if !slug.is_empty() => match donation_campaign_repo.find_by_slug(slug).await? {
            Some(c) if !c.is_active => {
                return Err(AppError::BadRequest(format!(
                    "Campaign '{}' is no longer accepting donations.",
                    c.name,
                )));
            }
            Some(c) => (Some(c.id), c.name),
            None => (None, "General donation".to_string()),
        },
        _ => (None, "General donation".to_string()),
    };

    // Email match → existing member? If yes, route through the
    // member-attributed donation flow so the donation appears in their
    // payment history. If no, public-donation flow with donor identity
    // captured on the payment row directly.
    let stripe_client = stripe_client.as_ref().ok_or_else(|| {
        AppError::ServiceUnavailable("Payment processing not configured".to_string())
    })?;

    let success_url = format!("{}/portal/payments/success", settings.server.base_url);
    let cancel_url = format!("{}/portal/payments/cancel", settings.server.base_url);

    let existing_member = member_repo.find_by_email(email).await?;

    let (checkout_url, payment_id) = match existing_member {
        Some(member) => {
            stripe_client
                .create_donation_checkout_session(
                    member.id,
                    &campaign_name,
                    campaign_id,
                    request.amount_cents,
                    success_url,
                    cancel_url,
                )
                .await?
        }
        None => {
            stripe_client
                .create_public_donation_checkout_session(
                    name,
                    email,
                    &campaign_name,
                    campaign_id,
                    request.amount_cents,
                    success_url,
                    cancel_url,
                )
                .await?
        }
    };

    Ok((
        StatusCode::OK,
        Json(PublicDonateResponse {
            payment_id,
            checkout_url,
        }),
    ))
}

#[cfg(test)]
mod announcement_markdown_tests {
    //! Public-feed rendering: `/public/announcements` carries a sanitized
    //! `content_html`, and the RSS item description carries the same
    //! sanitized rendered HTML. Both flow through the shared pipeline
    //! (`crate::util::markdown::render_announcement_markdown`).

    use super::*;
    use axum::{body::Body, http::Request, routing::get, Router};
    use chrono::Utc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::{
        domain::{Announcement, AnnouncementType, CreateMemberRequest, Member},
        repository::{MemberRepository, SqliteAnnouncementRepository, SqliteMemberRepository},
    };

    // A Markdown body exercising every relevant case: formatting that must
    // render, plus disallowed constructs that must be stripped.
    const RICH_BODY: &str = "**bold** *italic* ~~struck~~\n\n\
        A [safe link](https://example.com) and a [bad link](javascript:alert(1)).\n\n\
        <script>alert(2)</script>\n\n\
        ![img](https://example.com/x.png)";

    fn assert_sanitized(html: &str) {
        assert!(
            html.contains("<strong>bold</strong>"),
            "bold rendered: {html}"
        );
        assert!(html.contains("<em>italic</em>"), "italic rendered: {html}");
        assert!(
            html.contains("<del>struck</del>"),
            "strike rendered: {html}"
        );
        assert!(
            html.contains("href=\"https://example.com\""),
            "safe https link preserved: {html}"
        );
        assert!(!html.contains("<script"), "no live script element: {html}");
        assert!(
            !html.contains("javascript:"),
            "no javascript: scheme: {html}"
        );
        assert!(!html.contains("<img"), "no img element: {html}");
    }

    async fn migrated_pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn public_announcements_entry_carries_sanitized_content_html() {
        let pool = migrated_pool().await;

        let member_repo: Arc<dyn MemberRepository> =
            Arc::new(SqliteMemberRepository::new(pool.clone()));
        let member: Member = member_repo
            .create(CreateMemberRequest {
                email: "admin@example.com".to_string(),
                username: "admin".to_string(),
                full_name: "Admin".to_string(),
                password: "p4ssword_long_enough".to_string(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .unwrap();

        let announcement_repo: Arc<dyn AnnouncementRepository> =
            Arc::new(SqliteAnnouncementRepository::new(pool.clone()));
        let now = Utc::now();
        announcement_repo
            .create(Announcement {
                id: Uuid::new_v4(),
                title: "Rich".to_string(),
                content: RICH_BODY.to_string(),
                announcement_type: AnnouncementType::General,
                announcement_type_id: None,
                is_public: true,
                featured: false,
                image_url: None,
                published_at: Some(now),
                scheduled_publish_at: None,
                scheduled_publish_timezone: "UTC".to_string(),
                created_by: member.id,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let app = Router::new()
            .route("/public/announcements", get(list_announcements))
            .with_state(announcement_repo);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/public/announcements")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let entry = &json.as_array().expect("array")[0];

        let content_html = entry
            .get("content_html")
            .and_then(|v| v.as_str())
            .expect("content_html field present");
        assert_sanitized(content_html);

        // Raw Markdown source is kept alongside the rendered HTML.
        let raw = entry.get("content").and_then(|v| v.as_str()).unwrap();
        assert_eq!(raw, RICH_BODY, "raw content preserved");
    }

    #[tokio::test]
    async fn public_announcements_omit_internal_fields() {
        let pool = migrated_pool().await;

        let member_repo: Arc<dyn MemberRepository> =
            Arc::new(SqliteMemberRepository::new(pool.clone()));
        let member: Member = member_repo
            .create(CreateMemberRequest {
                email: "admin@example.com".to_string(),
                username: "admin".to_string(),
                full_name: "Admin".to_string(),
                password: "p4ssword_long_enough".to_string(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .unwrap();

        let announcement_repo: Arc<dyn AnnouncementRepository> =
            Arc::new(SqliteAnnouncementRepository::new(pool.clone()));
        let now = Utc::now();
        announcement_repo
            .create(Announcement {
                id: Uuid::new_v4(),
                title: "Public".to_string(),
                content: "Body".to_string(),
                announcement_type: AnnouncementType::General,
                announcement_type_id: None,
                is_public: true,
                featured: true,
                image_url: Some("https://example.com/x.png".to_string()),
                published_at: Some(now),
                scheduled_publish_at: Some(now),
                scheduled_publish_timezone: "America/New_York".to_string(),
                created_by: member.id,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let app = Router::new()
            .route("/public/announcements", get(list_announcements))
            .with_state(announcement_repo);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/public/announcements")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = json.as_array().expect("array");
        assert_eq!(arr.len(), 1, "one published public announcement");
        let entry = arr[0].as_object().expect("object");

        // Internal fields must NOT reach the anonymous marketing surface.
        for internal in [
            "created_by",
            "created_at",
            "updated_at",
            "announcement_type_id",
            "is_public",
            "scheduled_publish_at",
            "scheduled_publish_timezone",
        ] {
            assert!(
                !entry.contains_key(internal),
                "internal field `{internal}` leaked: {entry:?}",
            );
        }

        // The projected public field set is present and unchanged.
        for public in [
            "id",
            "title",
            "content",
            "content_html",
            "announcement_type",
            "featured",
            "image_url",
            "published_at",
        ] {
            assert!(
                entry.contains_key(public),
                "public field `{public}` missing: {entry:?}",
            );
        }
    }

    #[test]
    fn rss_description_carries_sanitized_rendered_html() {
        let now = Utc::now();
        let announcement = Announcement {
            id: Uuid::new_v4(),
            title: "Rich".to_string(),
            content: RICH_BODY.to_string(),
            announcement_type: AnnouncementType::General,
            announcement_type_id: None,
            is_public: true,
            featured: false,
            image_url: None,
            published_at: Some(now),
            scheduled_publish_at: None,
            scheduled_publish_timezone: "UTC".to_string(),
            created_by: Uuid::new_v4(),
            created_at: now,
            updated_at: now,
        };

        let rss = generate_rss_feed(&[announcement]);
        // The description block carries the sanitized rendered HTML.
        assert!(
            rss.contains("<description><![CDATA[") && rss.contains("<strong>bold</strong>"),
            "rss description carries rendered HTML: {rss}"
        );
        assert!(
            !rss.contains("<script"),
            "no live script element in rss: {rss}"
        );
        assert!(
            !rss.contains("javascript:"),
            "no javascript: scheme in rss: {rss}"
        );
    }
}
