use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{api::middleware::auth::CurrentUser, service::member_service::MemberService};

#[derive(Debug, Deserialize)]
pub struct UpdateDiscordIdForm {
    /// Empty string means "clear the discord_id".
    pub discord_id: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

/// Admin sets or clears a member's Discord snowflake ID. Validates the
/// format up-front; on success, fires a MemberUpdated event so Discord
/// integration can re-sync roles to the new ID (and strip them from
/// the old, if any).
pub async fn admin_update_discord_id(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<UpdateDiscordIdForm>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return discord_id_result(false, "Invalid member ID"),
    };

    let new_value = if form.discord_id.trim().is_empty() {
        None
    } else {
        Some(form.discord_id.clone())
    };

    match member_service
        .update_discord_id(current_user.member.id, id, new_value)
        .await
    {
        Ok(member) => {
            let msg = match &member.discord_id {
                Some(v) => format!("Discord ID set to {} (role sync triggered).", v),
                None => "Discord ID cleared.".to_string(),
            };
            discord_id_result(true, &msg)
        }
        Err(e) => discord_id_result(false, &format!("Failed to save: {}", e)),
    }
}

fn discord_id_result(ok: bool, detail: &str) -> axum::response::Response {
    let escaped = crate::web::escape_html(detail);
    let (bg, fg) = if ok {
        ("bg-green-50", "text-green-900")
    } else {
        ("bg-red-50", "text-red-900")
    };
    axum::response::Html(format!(
        r#"<div id="discord-id-result" class="mt-2 p-2 {bg} {fg} rounded text-sm">{detail}</div>"#,
        bg = bg,
        fg = fg,
        detail = escaped,
    ))
    .into_response()
}
