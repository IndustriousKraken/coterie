//! Syndication feeds: the RSS 2.0 announcement feed and the iCal event
//! feed, plus the XML/iCal escaping their string serialization needs.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::Utc;

use crate::{
    domain::{Announcement, Event, EventVisibility},
    error::Result,
    repository::{AnnouncementRepository, EventRepository},
};

use super::derive_utc_instants;

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

/// Escape text for use inside XML CDATA sections. The only sequence that
/// can break a CDATA block is `]]>`, which we split into two adjacent
/// CDATA sections: `]]]]><![CDATA[>`.
fn escape_cdata(s: &str) -> String {
    s.replace("]]>", "]]]]><![CDATA[>")
}

// Helper function to generate RSS feed
pub(super) fn generate_rss_feed(announcements: &[Announcement]) -> String {
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

// Helper function to generate iCal feed
// Private (MembersOnly) events are sanitized to show only time slot
pub(super) fn generate_ical_feed(events: &[Event]) -> String {
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
