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
    domain::{CreateMemberRequest, Event, Announcement, MemberStatus, MembershipType},
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
    
    // Validate password strength (minimum 8 characters)
    if request.password.len() < 8 {
        return Err(AppError::BadRequest("Password must be at least 8 characters".to_string()));
    }
    
    // Create member with Pending status
    let create_request = CreateMemberRequest {
        email: request.email,
        username: request.username,
        full_name: request.full_name,
        password: request.password,
        membership_type: request.membership_type.unwrap_or(MembershipType::Regular),
    };
    
    // Create the member
    let member = state.service_context.member_repo.create(create_request).await
        .map_err(|e| match e {
            AppError::Database(msg) if msg.contains("UNIQUE") => {
                if msg.contains("email") {
                    AppError::Conflict("Email already registered".to_string())
                } else if msg.contains("username") {
                    AppError::Conflict("Username already taken".to_string())
                } else {
                    AppError::Conflict("Registration failed: duplicate information".to_string())
                }
            },
            _ => e,
        })?;
    
    let response = SignupResponse {
        member_id: member.id,
        status: member.status,
        message: "Registration successful. Your account is pending approval.".to_string(),
    };
    
    Ok((StatusCode::CREATED, Json(response)))
}

pub async fn list_events(
    State(state): State<AppState>,
    Query(params): Query<PublicEventsQuery>,
) -> Result<Response> {
    // Get public events only
    let events = state.service_context.event_repo.list_public().await?;
    
    // Filter to upcoming events only
    let upcoming_events: Vec<Event> = events
        .into_iter()
        .filter(|e| e.start_time > Utc::now())
        .take(params.limit.unwrap_or(50) as usize)
        .collect();
    
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
    // Get public events
    let events = state.service_context.event_repo.list_public().await?;
    
    // Generate iCal format
    let ical = generate_ical_feed(&events);
    
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/calendar; charset=utf-8")],
        ical,
    ).into_response())
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
            rss.push_str(&format!("        <title><![CDATA[{}]]></title>\n", announcement.title));
            rss.push_str(&format!("        <description><![CDATA[{}]]></description>\n", announcement.content));
            rss.push_str(&format!("        <guid isPermaLink=\"false\">{}</guid>\n", announcement.id));
            rss.push_str(&format!("        <pubDate>{}</pubDate>\n", published.to_rfc2822()));
            rss.push_str("    </item>\n");
        }
    }
    
    rss.push_str("</channel>\n</rss>");
    rss
}

// Helper function to generate iCal feed
fn generate_ical_feed(events: &[Event]) -> String {
    let mut ical = String::from("BEGIN:VCALENDAR\r\n");
    ical.push_str("VERSION:2.0\r\n");
    ical.push_str("PRODID:-//Coterie//Events//EN\r\n");
    ical.push_str("CALSCALE:GREGORIAN\r\n");
    ical.push_str("METHOD:PUBLISH\r\n");
    ical.push_str("X-WR-CALNAME:Coterie Events\r\n");
    
    for event in events {
        ical.push_str("BEGIN:VEVENT\r\n");
        ical.push_str(&format!("UID:{}\r\n", event.id));
        ical.push_str(&format!("DTSTART:{}\r\n", event.start_time.format("%Y%m%dT%H%M%SZ")));
        
        if let Some(end_time) = event.end_time {
            ical.push_str(&format!("DTEND:{}\r\n", end_time.format("%Y%m%dT%H%M%SZ")));
        }
        
        ical.push_str(&format!("SUMMARY:{}\r\n", event.title));
        ical.push_str(&format!("DESCRIPTION:{}\r\n", event.description.replace('\n', "\\n")));
        
        if let Some(location) = &event.location {
            ical.push_str(&format!("LOCATION:{}\r\n", location));
        }
        
        ical.push_str(&format!("CREATED:{}\r\n", event.created_at.format("%Y%m%dT%H%M%SZ")));
        ical.push_str(&format!("LAST-MODIFIED:{}\r\n", event.updated_at.format("%Y%m%dT%H%M%SZ")));
        ical.push_str("STATUS:CONFIRMED\r\n");
        ical.push_str("END:VEVENT\r\n");
    }
    
    ical.push_str("END:VCALENDAR\r\n");
    ical
}