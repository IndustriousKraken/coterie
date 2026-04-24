use axum::{
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    api::state::AppState,
    domain::{CreateMemberRequest, Event, Announcement, EventVisibility, MemberStatus, MembershipType},
    error::{AppError, Result},
};

#[derive(Debug, Deserialize)]
pub struct SignupRequest {
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub password: String,
    pub membership_type: Option<MembershipType>,
}

#[derive(Debug, Serialize)]
pub struct SignupResponse {
    pub member_id: Uuid,
    pub status: MemberStatus,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct PublicEventsQuery {
    pub limit: Option<i64>,
    pub format: Option<String>, // "json" or "ical"
}

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
    
    // Create member with Pending status
    let create_request = CreateMemberRequest {
        email: request.email,
        username: request.username,
        full_name: request.full_name,
        password: request.password,
        membership_type: request.membership_type.unwrap_or(MembershipType::Regular),
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
        .get_value("organization.name")
        .await
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Coterie".to_string())
}

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

#[derive(Debug, Serialize)]
pub struct PrivateEventCount {
    pub count: i64,
}

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