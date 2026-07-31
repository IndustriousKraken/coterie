//! The public signup funnel: `POST /public/signup`, the payment-mode
//! abandoned-checkout retry, the verification email, and the Stripe
//! Checkout session creation shared by both the fresh-signup and
//! verify-time paths.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    api::{middleware::bot_challenge::BotChallengeVerifier, state::MoneyLimiter},
    auth::AuthService,
    config::Settings,
    domain::{
        configurable_types::MembershipTypeConfig, CreateMemberRequest, MemberStatus, PaymentStatus,
    },
    email::EmailSender,
    error::{AppError, Result},
    payments::StripeClient,
    repository::{MemberRepository, PaymentRepository},
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
    /// `bot_challenge.provider = "disabled"`. See the `bot_challenge`
    /// settings (migration 041) and `DynamicBotChallengeVerifier`.
    #[serde(default)]
    pub captcha_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SignupResponse {
    pub member_id: Uuid,
    pub status: MemberStatus,
    pub message: String,
    /// Present only on the payment-mode abandoned-checkout retry (an
    /// existing, email-verified Pending member proving their password):
    /// redirect the browser here to resume/complete the Stripe Checkout.
    /// Fresh signups never carry it — the Stripe surface is deferred until
    /// the member verifies their email (then the verify handler initiates
    /// checkout and redirects to it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkout_url: Option<String>,
}

#[utoipa::path(
    post,
    path = "/public/signup",
    tag = "public",
    request_body = SignupRequest,
    responses(
        (status = 201, description = "Member created; verification email sent. No \
            `checkout_url` — in payment mode the Stripe Checkout is initiated only after \
            the member verifies their email", body = SignupResponse),
        (status = 200, description = "Payment-mode retry: the email belongs to a Pending \
            signup with no completed payment and the password verified. For a verified \
            member a `checkout_url` resumes/creates the session; for an unverified member \
            the verification email is re-queued and no `checkout_url` is returned", body = SignupResponse),
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
    if !money_limiter.0.check_and_record(ip, "public.signup") {
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
    if let Err(rule) = crate::auth::validate_password_logged(&request.password, None, Some(ip)) {
        return Err(AppError::BadRequest(rule.message()));
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
                    email_sender.as_ref(),
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
                "Registration failed: an account with this information already exists".to_string(),
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

    // Signup no longer reaches Stripe: a payment-mode signup creates the
    // Pending member and queues the verification email, but the Stripe
    // customer + Checkout session are deferred until the member verifies
    // their email (see `initiate_checkout_on_verify`, called from the
    // verify handler). Gating the Stripe surface behind a verifiable
    // inbox is what stops automated card-testing signups from ever
    // reaching checkout. Fresh signup therefore never carries a
    // checkout URL; only the abandoned-checkout retry does.
    let message = if signup_mode == SignupMode::Payment {
        "Registration successful. Please check your email to verify your address and \
         continue to payment."
            .to_string()
    } else {
        "Registration successful. Please check your email to verify your account.".to_string()
    };

    let response = SignupResponse {
        member_id: member.id,
        status: member.status,
        message,
        checkout_url: None,
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

/// Initiate a signup checkout at email-verification time, when the funnel
/// requires it. The verify handler calls this AFTER marking the email
/// verified, so the "a Stripe Checkout session is only ever created for a
/// verified member" invariant holds at this single call site.
///
/// Returns `Ok(Some(url))` when a Stripe Checkout session was created —
/// payment mode, member still `Pending`, no completed membership payment,
/// a paid membership type, and Stripe configured. Returns `Ok(None)` when
/// the member stays in the approval funnel (approval mode, a fee-0 type,
/// an already-paid member, or Stripe not configured); the caller then keeps
/// the "verified; awaiting review" result. Uses the same
/// `create_signup_checkout` contract (metadata + `signup_auto_renew`) as
/// every other signup checkout.
pub(crate) async fn initiate_checkout_on_verify(
    payment_repo: &dyn PaymentRepository,
    membership_type_service: &MembershipTypeService,
    stripe_client: Option<&StripeClient>,
    settings_service: &SettingsService,
    settings: &Settings,
    member: &crate::domain::Member,
) -> Result<Option<String>> {
    if settings_service.signup_mode().await != SignupMode::Payment {
        return Ok(None);
    }
    if member.status != MemberStatus::Pending {
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
    let Some(membership_type) = membership_type_service
        .get(member.membership_type_id)
        .await?
    else {
        return Ok(None);
    };
    if membership_type.fee_cents <= 0 {
        return Ok(None);
    }
    let Some(stripe_client) = stripe_client else {
        tracing::error!(
            "signup_mode=payment but Stripe is not configured; member {} verified \
             without a checkout session",
            member.id,
        );
        return Ok(None);
    };
    let url = create_signup_checkout(
        stripe_client,
        settings_service,
        settings,
        member,
        &membership_type,
    )
    .await?;
    Ok(Some(url))
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
    email_sender: &dyn EmailSender,
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
    let Some((_, hash)) = crate::auth::get_password_hash(db_pool, email).await? else {
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

    // Email-verified gate — the retry path must not become an unverified
    // back door to Stripe. A bot sets (and therefore knows) its own
    // password, so proving the password alone can't be enough: only an
    // email-verified member may reach checkout. An unverified Pending
    // member with the correct password gets the verification email
    // re-queued and NO checkout URL, mirroring what fresh signup now does.
    // (Wrong password / paid / non-Pending already returned above, so
    // this 200 only ever answers the member's own account.)
    if !member.email_verified() {
        if let Err(e) =
            send_verification_email(db_pool, settings, settings_service, email_sender, &member)
                .await
        {
            tracing::error!(
                "Signup retry: re-queue verification email failed for member {}: {}",
                member.id,
                e,
            );
        }
        return Ok(Some((
            StatusCode::OK,
            Json(SignupResponse {
                member_id: member.id,
                status: member.status,
                message: "Please check your email to verify your address before continuing \
                          to payment."
                    .to_string(),
                checkout_url: None,
            }),
        )));
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
