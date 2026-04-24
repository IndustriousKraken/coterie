//! Admin page for browsing the audit log. Backs onto AuditService;
//! supports a simple filter + pagination via `before` cursor and
//! `limit` query params.

use askama::Template;
use axum::{
    extract::{Query, State},
    response::{IntoResponse, Response, Redirect},
    Extension,
};
use serde::Deserialize;

use crate::{
    api::{middleware::auth::CurrentUser, state::AppState},
    web::templates::{HtmlTemplate, UserInfo},
};
use crate::web::portal::is_admin;

#[derive(Template)]
#[template(path = "admin/audit_log.html")]
pub struct AuditLogTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub entries: Vec<AuditEntryDisplay>,
    pub action_filter: String,
    pub actor_filter: String,
    pub limit: i64,
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
    #[serde(default)]
    pub limit: Option<i64>,
}

pub async fn audit_log_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Query(query): Query<AuditLogQuery>,
) -> Response {
    if !is_admin(&current_user.member) {
        return Redirect::to("/portal/dashboard").into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let limit = query.limit.unwrap_or(100).clamp(10, 500);

    // Fetch more than we'll show and filter in Rust. For a small DB
    // this is simpler than baking filters into the AuditService API;
    // we can revisit if audit volume grows.
    let raw = state.service_context.audit_service
        .recent(limit * 3) // over-fetch a bit to account for filtering
        .await
        .unwrap_or_default();

    let action_filter = query.action.to_lowercase();
    let actor_filter = query.actor.to_lowercase();

    let entries: Vec<AuditEntryDisplay> = raw.into_iter()
        .filter(|e| action_filter.is_empty() || e.action.to_lowercase().contains(&action_filter))
        .filter(|e| {
            actor_filter.is_empty()
                || e.actor_name.as_deref().unwrap_or("").to_lowercase().contains(&actor_filter)
        })
        .take(limit as usize)
        .map(|e| AuditEntryDisplay {
            actor: e.actor_name.clone().unwrap_or_else(|| "(system)".to_string()),
            action: pretty_action(&e.action),
            entity: format!("{} {}", e.entity_type, short_id(&e.entity_id)),
            detail: e.new_value.clone().unwrap_or_default(),
            when: e.created_at.format("%b %d, %Y at %H:%M UTC").to_string(),
        })
        .collect();

    HtmlTemplate(AuditLogTemplate {
        current_user: Some(user_info),
        is_admin: true,
        entries,
        action_filter: query.action,
        actor_filter: query.actor,
        limit,
    }).into_response()
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
