//! Admin page for browsing the audit log. Backs onto AuditService;
//! supports a simple filter + pagination via `before` cursor and
//! `limit` query params.

use askama::Template;
use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use chrono::Utc;
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    web::templates::{BaseContext, HtmlTemplate},
};

#[derive(Template)]
#[template(path = "admin/audit_log.html")]
pub struct AuditLogTemplate {
    pub base: BaseContext,
    pub entries: Vec<AuditEntryDisplay>,
    pub action_filter: String,
    pub actor_filter: String,
    pub target_filter: String,
    pub limit: i64,
    /// Query string to preserve filters on the CSV-export link.
    pub export_qs: String,
}

pub struct AuditEntryDisplay {
    pub actor: String,
    pub action: String,
    pub entity: String,
    pub detail: String,
    pub when: String,
}

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub actor: String,
    /// Free-text search against entity_id (matches member UUIDs,
    /// setting keys, etc.).
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub limit: Option<i64>,
}

pub async fn audit_log_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
    Query(query): Query<AuditLogQuery>,
) -> Response {
    let limit = query.limit.unwrap_or(100).clamp(10, 500);
    let entries = filtered_entries(&state, &query, limit).await;
    let export_qs = build_export_qs(&query);

    HtmlTemplate(AuditLogTemplate {
        base: BaseContext::for_member(&state, &current_user, &session).await,
        entries,
        action_filter: query.action,
        actor_filter: query.actor,
        target_filter: query.target,
        limit,
        export_qs,
    }).into_response()
}

/// Apply filters over the recent audit entries. Used by both the HTML
/// page and the CSV exporter so the two stay consistent.
async fn filtered_entries(
    state: &AppState,
    query: &AuditLogQuery,
    limit: i64,
) -> Vec<AuditEntryDisplay> {
    let raw = state.service_context.audit_service
        .recent(limit * 3) // over-fetch a bit to account for filtering
        .await
        .unwrap_or_default();

    let action_filter = query.action.to_lowercase();
    let actor_filter = query.actor.to_lowercase();
    let target_filter = query.target.to_lowercase();

    raw.into_iter()
        .filter(|e| action_filter.is_empty() || e.action.to_lowercase().contains(&action_filter))
        .filter(|e| {
            actor_filter.is_empty()
                || e.actor_name.as_deref().unwrap_or("").to_lowercase().contains(&actor_filter)
        })
        .filter(|e| {
            target_filter.is_empty()
                || e.entity_id.to_lowercase().contains(&target_filter)
        })
        .take(limit as usize)
        .map(|e| AuditEntryDisplay {
            actor: e.actor_name.clone().unwrap_or_else(|| "(system)".to_string()),
            action: pretty_action(&e.action),
            entity: format!("{} {}", e.entity_type, short_id(&e.entity_id)),
            detail: format_detail(e.old_value.as_deref(), e.new_value.as_deref()),
            when: e.created_at.format("%b %d, %Y at %H:%M UTC").to_string(),
        })
        .collect()
}

/// Format the detail column. If both old and new are present, show a
/// compact "X → Y" diff; otherwise fall back to whichever side exists.
fn format_detail(old: Option<&str>, new: Option<&str>) -> String {
    match (old, new) {
        (Some(o), Some(n)) if !o.is_empty() && !n.is_empty() => {
            format!("{} → {}", truncate(o), truncate(n))
        }
        (_, Some(n)) if !n.is_empty() => truncate(n).to_string(),
        (Some(o), _) if !o.is_empty() => truncate(o).to_string(),
        _ => String::new(),
    }
}

fn truncate(s: &str) -> &str {
    const MAX: usize = 120;
    if s.len() <= MAX { s } else { &s[..MAX] }
}

fn build_export_qs(q: &AuditLogQuery) -> String {
    let mut parts = Vec::new();
    if !q.action.is_empty() {
        parts.push(format!("action={}", urlencoding::encode(&q.action)));
    }
    if !q.actor.is_empty() {
        parts.push(format!("actor={}", urlencoding::encode(&q.actor)));
    }
    if !q.target.is_empty() {
        parts.push(format!("target={}", urlencoding::encode(&q.target)));
    }
    if let Some(l) = q.limit {
        parts.push(format!("limit={}", l));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

/// Export audit log as CSV. Respects the same filters as the page view.
pub async fn audit_log_export(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Query(query): Query<AuditLogQuery>,
) -> Response {
    // Export is less bounded than the UI view — default to 5000, cap
    // at 50k. Admins doing an annual compliance dump will want the
    // whole table; anyone needing more can adjust retention or dump
    // the DB directly.
    let limit = query.limit.unwrap_or(5000).clamp(1, 50_000);

    let raw = state.service_context.audit_service
        .recent(limit * 3)
        .await
        .unwrap_or_default();

    let action_filter = query.action.to_lowercase();
    let actor_filter = query.actor.to_lowercase();
    let target_filter = query.target.to_lowercase();

    let rows = raw.into_iter()
        .filter(|e| action_filter.is_empty() || e.action.to_lowercase().contains(&action_filter))
        .filter(|e| actor_filter.is_empty()
            || e.actor_name.as_deref().unwrap_or("").to_lowercase().contains(&actor_filter))
        .filter(|e| target_filter.is_empty() || e.entity_id.to_lowercase().contains(&target_filter))
        .take(limit as usize);

    // Minimal hand-rolled CSV writer. The fields we emit (UUIDs,
    // action tags, ISO timestamps, plain-text values) are well-behaved
    // for the most part, but any of the *_value fields could contain
    // commas/quotes/newlines from member-entered text, so we quote
    // everything defensively.
    let mut out = String::with_capacity(16 * 1024);
    out.push_str("timestamp,actor_id,actor_name,action,entity_type,entity_id,old_value,new_value,ip_address\n");
    for e in rows {
        push_csv(&mut out, &e.created_at.to_rfc3339());
        out.push(',');
        push_csv(&mut out, &e.actor_id.map(|u| u.to_string()).unwrap_or_default());
        out.push(',');
        push_csv(&mut out, e.actor_name.as_deref().unwrap_or(""));
        out.push(',');
        push_csv(&mut out, &e.action);
        out.push(',');
        push_csv(&mut out, &e.entity_type);
        out.push(',');
        push_csv(&mut out, &e.entity_id);
        out.push(',');
        push_csv(&mut out, e.old_value.as_deref().unwrap_or(""));
        out.push(',');
        push_csv(&mut out, e.new_value.as_deref().unwrap_or(""));
        out.push(',');
        push_csv(&mut out, e.ip_address.as_deref().unwrap_or(""));
        out.push('\n');
    }

    let filename = format!("coterie-audit-{}.csv", Utc::now().format("%Y-%m-%d"));
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename)),
        ],
        out,
    ).into_response()
}

/// Emit a single CSV field. We always quote — simpler than deciding
/// when we need to — and escape embedded quotes per RFC 4180.
fn push_csv(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        if c == '"' {
            out.push('"');
            out.push('"');
        } else {
            out.push(c);
        }
    }
    out.push('"');
}

fn pretty_action(action: &str) -> String {
    action.replace('_', " ")
}

fn short_id(id: &str) -> String {
    // UUIDs are 36 chars; show the first 8 to keep the table readable.
    if id.len() > 8 {
        format!("{}…", &id[..8])
    } else {
        id.to_string()
    }
}
