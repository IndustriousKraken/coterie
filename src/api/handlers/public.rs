use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    api::state::AppState,
    domain::{CreateMemberRequest, Event, Announcement, EventVisibility, MemberStatus},
    error::{AppError, Result},
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
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SignupResponse {
    pub member_id: Uuid,
    pub status: MemberStatus,
    pub message: String,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct PublicEventsQuery {
    /// Maximum number of upcoming events to return (default 50).
    pub limit: Option<i64>,
    /// Response format: omit or `"json"` for JSON; `"ical"` for an
    /// iCal/.ics calendar feed.
    pub format: Option<String>,
}

#[utoipa::path(
    post,
    path = "/public/signup",
    tag = "public",
    request_body = SignupRequest,
    responses(
        (status = 201, description = "Member created; verification email sent", body = SignupResponse),
        (status = 400, description = "Invalid email or weak password"),
        (status = 409, description = "Email or username already in use"),
    ),
)]
pub async fn signup(
    State(state): State<AppState>,
    Json(request): Json<SignupRequest>,
) -> Result<(StatusCode, Json<SignupResponse>)> {
    // Validate email format
    if !request.email.contains('@') {
        return Err(AppError::BadRequest("Invalid email format".to_string()));
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
            let mt = state.service_context.membership_type_service
                .get_by_slug(slug)
                .await?
                .ok_or_else(|| AppError::BadRequest(format!(
                    "Unknown membership type slug: {}", slug,
                )))?;
            Some(mt.id)
        }
        None => None,
    };

    // Create member with Pending status
    let create_request = CreateMemberRequest {
        email: request.email,
        username: request.username,
        full_name: request.full_name,
        password: request.password,
        membership_type_id,
    };
    
    // Create the member. Use a generic error for UNIQUE violations to
    // prevent attackers from enumerating valid emails/usernames.
    let member = state.service_context.member_repo.create(create_request).await
        .map_err(|e| match e {
            AppError::Database(msg) if msg.contains("UNIQUE") => {
                AppError::Conflict("Registration failed: an account with this information already exists".to_string())
            },
            _ => e,
        })?;

    // Send email verification. Soft-fail on send error: the account is
    // already created and an admin can manually verify / resend later.
    if let Err(e) = send_verification_email(&state, &member).await {
        tracing::error!(
            "Signup succeeded but verification email failed for member {}: {}",
            member.id, e
        );
    }

    let response = SignupResponse {
        member_id: member.id,
        status: member.status,
        message: "Registration successful. Please check your email to verify your account.".to_string(),
    };

    Ok((StatusCode::CREATED, Json(response)))
}

/// Generate a verification token and email the link to the member.
async fn send_verification_email(
    state: &AppState,
    member: &crate::domain::Member,
) -> Result<()> {
    use crate::{auth::EmailTokenService, email::{self, templates::{VerifyHtml, VerifyText}}};

    let service = EmailTokenService::verification(state.service_context.db_pool.clone());
    let created = service.create(member.id, chrono::Duration::hours(24)).await?;

    let verify_url = format!(
        "{}/verify?token={}",
        state.settings.server.base_url.trim_end_matches('/'),
        created.token,
    );
    let org_name = org_name(state).await;
    let html = VerifyHtml { full_name: &member.full_name, org_name: &org_name, verify_url: &verify_url };
    let text = VerifyText { full_name: &member.full_name, org_name: &org_name, verify_url: &verify_url };
    let message = email::message_from_templates(
        member.email.clone(),
        format!("Verify your email for {}", org_name),
        &html,
        &text,
    )?;
    state.service_context.email_sender.send(&message).await
}

/// Look up the configured organization name from settings, falling back
/// to "Coterie" if unset.
async fn org_name(state: &AppState) -> String {
    state.service_context.settings_service
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
        (status = 200, description = "Upcoming public + sanitized members-only events", body = [Event],
            content_type = "application/json"),
        (status = 200, description = "iCal feed (when format=ical)", content_type = "text/calendar"),
    ),
)]
pub async fn list_events(
    State(state): State<AppState>,
    Query(params): Query<PublicEventsQuery>,
) -> Result<Response> {
    // Get public events (full details)
    let public_events = state.service_context.event_repo.list_public().await?;

    // Get members-only events (will be sanitized)
    let private_events = state.service_context.event_repo.list_members_only().await?;

    // Combine and filter to upcoming events only
    let now = Utc::now();
    let mut upcoming_events: Vec<Event> = public_events
        .into_iter()
        .chain(private_events.into_iter().map(|mut e| {
            // Sanitize private events
            e.title = "Members-Only Event".to_string();
            e.description = "This event is for members only. Log in to the portal to see details.".to_string();
            e.location = None;
            e.image_url = None;
            e
        }))
        .filter(|e| e.start_time > now)
        .collect();

    // Sort by start time
    upcoming_events.sort_by(|a, b| a.start_time.cmp(&b.start_time));

    // Apply limit
    upcoming_events.truncate(params.limit.unwrap_or(50) as usize);

    // Check if iCal format is requested
    if params.format.as_deref() == Some("ical") {
        let ical = generate_ical_feed(&upcoming_events);
        Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/calendar; charset=utf-8")],
            ical,
        ).into_response())
    } else {
        Ok(Json(upcoming_events).into_response())
    }
}

#[utoipa::path(
    get,
    path = "/public/announcements",
    tag = "public",
    responses(
        (status = 200, description = "Published public announcements", body = [Announcement]),
    ),
)]
pub async fn list_announcements(
    State(state): State<AppState>,
) -> Result<Json<Vec<Announcement>>> {
    // Get public announcements only
    let announcements = state.service_context.announcement_repo.list_public().await?;
    
    // Filter to published announcements only
    let published: Vec<Announcement> = announcements
        .into_iter()
        .filter(|a| a.published_at.is_some())
        .collect();
    
    Ok(Json(published))
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
    State(state): State<AppState>,
) -> Result<Response> {
    // Get recent public announcements
    let announcements = state.service_context.announcement_repo.list_public().await?;
    
    // Generate RSS XML
    let rss = generate_rss_feed(&announcements);
    
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/rss+xml; charset=utf-8")],
        rss,
    ).into_response())
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
pub async fn calendar_feed(
    State(state): State<AppState>,
) -> Result<Response> {
    // Get public events (full details)
    let public_events = state.service_context.event_repo.list_public().await?;

    // Get members-only events (will be sanitized in feed)
    let private_events = state.service_context.event_repo.list_members_only().await?;

    // Combine all events for the calendar
    let all_events: Vec<_> = public_events.into_iter()
        .chain(private_events.into_iter())
        .collect();

    // Generate iCal format (private events will be sanitized)
    let ical = generate_ical_feed(&all_events);

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/calendar; charset=utf-8")],
        ical,
    ).into_response())
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
    State(state): State<AppState>,
) -> Result<Json<PrivateEventCount>> {
    let count = state.service_context.event_repo.count_members_only_upcoming().await?;
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
    let mut rss = String::from(r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom">
<channel>
    <title>Coterie Announcements</title>
    <link>https://example.com/announcements</link>
    <description>Latest announcements from Coterie</description>
    <language>en-us</language>
    <lastBuildDate>"#);

    rss.push_str(&Utc::now().to_rfc2822());
    rss.push_str("</lastBuildDate>\n");

    for announcement in announcements.iter().take(20) {
        if let Some(published) = announcement.published_at {
            rss.push_str("    <item>\n");
            rss.push_str(&format!("        <title><![CDATA[{}]]></title>\n", escape_cdata(&announcement.title)));
            rss.push_str(&format!("        <description><![CDATA[{}]]></description>\n", escape_cdata(&announcement.content)));
            rss.push_str(&format!("        <guid isPermaLink=\"false\">{}</guid>\n", announcement.id));
            rss.push_str(&format!("        <pubDate>{}</pubDate>\n", published.to_rfc2822()));
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
        ical.push_str(&format!("DTSTART:{}\r\n", event.start_time.format("%Y%m%dT%H%M%SZ")));

        if let Some(end_time) = event.end_time {
            ical.push_str(&format!("DTEND:{}\r\n", end_time.format("%Y%m%dT%H%M%SZ")));
        }

        if is_private {
            // Sanitize private events - show only that something is happening
            ical.push_str("SUMMARY:Members-Only Event\r\n");
            ical.push_str("DESCRIPTION:This event is for members only. Log in to the portal to see details.\r\n");
        } else {
            ical.push_str(&format!("SUMMARY:{}\r\n", escape_ical_text(&event.title)));
            ical.push_str(&format!("DESCRIPTION:{}\r\n", escape_ical_text(&event.description)));

            if let Some(location) = &event.location {
                ical.push_str(&format!("LOCATION:{}\r\n", escape_ical_text(location)));
            }
        }

        ical.push_str(&format!("CREATED:{}\r\n", event.created_at.format("%Y%m%dT%H%M%SZ")));
        ical.push_str(&format!("LAST-MODIFIED:{}\r\n", event.updated_at.format("%Y%m%dT%H%M%SZ")));
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
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PublicDonateRequest>,
) -> Result<(StatusCode, Json<PublicDonateResponse>)> {
    // Rate limit by client IP. Public endpoint with payment side-effects
    // is the prime card-testing target — the limiter caps each IP at
    // 10 attempts per minute.
    let ip = crate::api::state::client_ip(
        &headers,
        state.settings.server.trust_forwarded_for(),
    );
    if !state.money_limiter.check_and_record(ip) {
        return Err(AppError::TooManyRequests);
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
        Some(slug) if !slug.is_empty() => {
            match state.service_context.donation_campaign_repo
                .find_by_slug(slug).await?
            {
                Some(c) if !c.is_active => {
                    return Err(AppError::BadRequest(format!(
                        "Campaign '{}' is no longer accepting donations.",
                        c.name,
                    )));
                }
                Some(c) => (Some(c.id), c.name),
                None => (None, "General donation".to_string()),
            }
        }
        _ => (None, "General donation".to_string()),
    };

    // Email match → existing member? If yes, route through the
    // member-attributed donation flow so the donation appears in their
    // payment history. If no, public-donation flow with donor identity
    // captured on the payment row directly.
    let stripe_client = state.stripe_client.as_ref()
        .ok_or_else(|| AppError::ServiceUnavailable(
            "Payment processing not configured".to_string()
        ))?;

    let success_url = format!("{}/portal/payments/success", state.settings.server.base_url);
    let cancel_url = format!("{}/portal/payments/cancel", state.settings.server.base_url);

    let existing_member = state.service_context.member_repo
        .find_by_email(email).await?;

    let (checkout_url, payment_id) = match existing_member {
        Some(member) => {
            stripe_client.create_donation_checkout_session(
                member.id,
                &campaign_name,
                campaign_id,
                request.amount_cents,
                success_url,
                cancel_url,
            ).await?
        }
        None => {
            stripe_client.create_public_donation_checkout_session(
                name,
                email,
                &campaign_name,
                campaign_id,
                request.amount_cents,
                success_url,
                cancel_url,
            ).await?
        }
    };

    Ok((StatusCode::OK, Json(PublicDonateResponse {
        payment_id,
        checkout_url,
    })))
}